//! Hash / optional ONNX-style ML backends for learned signals.
//! Enabled only with Cargo feature `ml`.

use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const DEFAULT_DIM: usize = 64;

/// Deterministic bag-of-tokens embedding (CI-friendly, no weight files).
pub fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let dim = dim.max(8);
    let mut v = vec![0.0f32; dim];
    for tok in text.split_whitespace() {
        let t = tok.to_ascii_lowercase();
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        let idx = (h.finish() as usize) % dim;
        v[idx] += 1.0;
        // Second hash for sign.
        let mut h2 = DefaultHasher::new();
        (t.as_str(), 1u8).hash(&mut h2);
        if h2.finish().is_multiple_of(2) {
            v[idx] *= -1.0;
        }
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for x in &mut v {
        *x /= norm;
    }
    v
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom < 1e-9 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

pub fn dim_from(extra: &serde_json::Map<String, Value>, catalog_dim: Option<usize>) -> usize {
    extra
        .get("dim")
        .and_then(|v| v.as_u64())
        .map(|u| u as usize)
        .or(catalog_dim)
        .unwrap_or(DEFAULT_DIM)
}

/// Score text against candidate strings; return best cosine and whether ≥ threshold.
pub fn embed_match(
    text: &str,
    candidates: &[String],
    threshold: f32,
    aggregation: &str,
    dim: usize,
) -> (bool, f32) {
    if candidates.is_empty() {
        return (false, 0.0);
    }
    let q = hash_embed(text, dim);
    let scores: Vec<f32> = candidates
        .iter()
        .map(|c| cosine(&q, &hash_embed(c, dim)))
        .collect();
    let conf = match aggregation {
        "max" | "any" => scores.iter().cloned().fold(0.0_f32, f32::max),
        _ => scores.iter().sum::<f32>() / scores.len() as f32,
    };
    (conf >= threshold, conf.clamp(0.0, 1.0))
}

/// Classifier-like: similarity of text to description / patterns / examples.
pub fn classify_match(
    text: &str,
    extra: &serde_json::Map<String, Value>,
    dim: usize,
) -> (bool, f32) {
    let threshold = extra
        .get("threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.35) as f32;
    let mut candidates: Vec<String> = vec![];
    if let Some(d) = extra.get("description").and_then(|v| v.as_str()) {
        candidates.push(d.to_string());
    }
    for key in [
        "examples",
        "candidates",
        "hard",
        "easy",
        "jailbreak_patterns",
        "benign_patterns",
    ] {
        if let Some(arr) = extra.get(key).and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    candidates.push(s.to_string());
                } else if let Some(obj) = v.as_object() {
                    if let Some(s) = obj.get("text").and_then(|t| t.as_str()) {
                        candidates.push(s.to_string());
                    }
                }
            }
        }
    }
    if let Some(cats) = extra.get("mmlu_categories").and_then(|v| v.as_array()) {
        for v in cats {
            if let Some(s) = v.as_str() {
                candidates.push(s.to_string());
            }
        }
    }
    if candidates.is_empty() {
        // Pattern-only: token overlap heuristic on keywords in extra.
        if let Some(pats) = extra.get("pii_types_allowed").and_then(|v| v.as_array()) {
            for v in pats {
                if let Some(s) = v.as_str() {
                    candidates.push(s.to_string());
                }
            }
        }
    }
    if candidates.is_empty() {
        // No anchors → low-confidence non-match (still evaluable under ml).
        return (false, 0.0);
    }
    embed_match(text, &candidates, threshold, "max", dim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_phrases_score_high() {
        let (ok, c) = embed_match(
            "installation guide troubleshooting",
            &["installation guide".into(), "troubleshooting steps".into()],
            0.2,
            "max",
            64,
        );
        assert!(ok, "conf={c}");
        assert!(c > 0.2);
    }

    #[test]
    fn hash_embed_negative_sign_and_empty_cosine() {
        // Hashing many tokens reliably hits the "even hash → negate" branch.
        let bag: Vec<String> = (0..32).map(|i| format!("tok{i}")).collect();
        let v = hash_embed(&bag.join(" "), 64);
        assert!(v.iter().any(|x| *x < 0.0));
        // Fewer dims than the minimum are clamped up, not truncated.
        assert_eq!(hash_embed("a", 2).len(), 8);

        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[0.0; 4], &[0.0; 4]), 0.0);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn embed_match_empty_candidates_and_mean_aggregation() {
        let (ok, c) = embed_match("anything", &[], 0.1, "max", 64);
        assert!(!ok);
        assert_eq!(c, 0.0);

        let (ok, _) = embed_match("alpha beta", &["alpha beta".into()], 0.1, "mean", 64);
        assert!(ok);
    }

    #[test]
    fn classify_match_description_examples_categories_and_fallbacks() {
        let map = |v: Value| v.as_object().unwrap().clone();

        let (ok, _) = classify_match(
            "prove the theorem",
            &map(serde_json::json!({
                "description": "prove the theorem",
                "examples": ["prove the theorem", {"text": "prove the theorem"}],
                "hard": ["prove the theorem"],
                "easy": ["say hi"],
                "jailbreak_patterns": ["prove the theorem"],
                "benign_patterns": ["say hi"]
            })),
            64,
        );
        assert!(ok);

        let (ok, _) = classify_match(
            "name and address",
            &map(serde_json::json!({"pii_types_allowed": ["name", "address"]})),
            64,
        );
        assert!(ok);

        // Array entries that are neither strings nor `{text: ...}` objects are skipped.
        let (ok, _) = classify_match(
            "prove the theorem",
            &map(serde_json::json!({
                "description": "prove the theorem",
                "examples": [{"no_text": 1}, 7, "prove the theorem"]
            })),
            64,
        );
        assert!(ok);

        let (ok, _) = classify_match(
            "math",
            &map(serde_json::json!({"mmlu_categories": ["math", "physics"]})),
            64,
        );
        assert!(ok);

        // No anchors at all → evaluable but non-matching.
        let (ok, c) = classify_match("anything", &map(serde_json::json!({})), 64);
        assert!(!ok);
        assert_eq!(c, 0.0);
    }
}
