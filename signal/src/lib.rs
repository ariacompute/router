//! Heuristic (and gated learned) signal extraction.

#[cfg(feature = "ml")]
mod ml;

use aria_router_config::{Recipe, RouterDocument, Signals};
use aria_router_core::{ChatRequest, RouterError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalHit {
    pub kind: String,
    pub name: String,
    pub matched: bool,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SignalSet {
    pub hits: Vec<SignalHit>,
}

impl SignalSet {
    pub fn get(&self, kind: &str, name: &str) -> Option<&SignalHit> {
        self.hits.iter().find(|h| h.kind == kind && h.name == name)
    }

    pub fn matched(&self, kind: &str, name: &str) -> bool {
        self.get(kind, name).is_some_and(|h| h.matched)
    }
}

/// Reject learned-signal assets this build cannot honour.
///
/// `weight_path` means ONNX weights, which are never loaded; ignoring it silently
/// would violate the repo contract (`AGENTS.md`: unimplemented YAML capability →
/// `Unsupported` at validate *and* runtime).
fn reject_unsupported_assets(s: &Signals) -> Result<(), RouterError> {
    for (key, val) in &s.extra {
        if aria_router_config::canonical_learned(key).is_none() {
            continue;
        }
        let items: Vec<&serde_json::Value> = match val {
            serde_json::Value::Array(a) => a.iter().collect(),
            serde_json::Value::Object(_) => vec![val],
            _ => continue,
        };
        for item in items {
            if item.get("weight_path").is_some_and(|v| !v.is_null()) {
                return Err(RouterError::Unsupported(format!(
                    "learned signal {key}: weight_path (ONNX weights) is not implemented"
                )));
            }
        }
    }
    Ok(())
}

pub fn extract(
    doc: &RouterDocument,
    recipe: &Recipe,
    req: &ChatRequest,
    metadata: &HashMap<String, String>,
) -> Result<SignalSet, RouterError> {
    let Some(routing) = &recipe.routing else {
        return Ok(SignalSet::default());
    };
    let needed = referenced(&routing.decisions);
    reject_unsupported_assets(&routing.signals)?;
    #[cfg(not(feature = "ml"))]
    if let Some(kind) = needed
        .iter()
        .find(|k| aria_router_config::canonical_learned(k).is_some())
    {
        return Err(RouterError::Unsupported(format!(
            "learned signal {kind} requires feature ml / weights"
        )));
    }
    let prompt = req.prompt_text();
    let last_user = req.last_user_text();
    let mut hits = vec![];
    let s = &routing.signals;
    if needed.iter().any(|k| k == "keyword") {
        hits.extend(eval_keywords(s, &last_user));
    }
    if needed.iter().any(|k| k == "language") {
        hits.extend(eval_language(s, &last_user));
    }
    if needed.iter().any(|k| k == "context") {
        hits.extend(eval_context(s, &prompt));
    }
    if needed.iter().any(|k| k == "authz") {
        hits.extend(eval_authz(s, metadata));
    }
    if needed.iter().any(|k| k == "conversation") {
        hits.extend(eval_conversation(s, req));
    }
    if needed.iter().any(|k| k == "metadata") {
        hits.extend(eval_metadata(s, metadata));
    }
    if needed.iter().any(|k| k == "event") {
        hits.extend(eval_event(s, &last_user));
    }
    if needed.iter().any(|k| k == "structure") {
        hits.extend(eval_structure(s, &last_user));
    }
    #[cfg(feature = "ml")]
    {
        hits.extend(eval_learned(doc, s, &needed, &last_user)?);
    }
    #[cfg(not(feature = "ml"))]
    {
        let _ = doc;
    }
    Ok(SignalSet { hits })
}

#[cfg(feature = "ml")]
fn eval_learned(
    doc: &RouterDocument,
    s: &Signals,
    needed: &[String],
    text: &str,
) -> Result<Vec<SignalHit>, RouterError> {
    use aria_router_config::canonical_learned;
    let mut hits = vec![];
    let fallback_dim = doc.global.model_catalog.values().find_map(|e| e.dim);
    // Iterate the *referenced* families so the emitted `kind` is exactly the
    // spelling used by the decision (plural keys included).
    for kind in needed {
        let Some(canon) = canonical_learned(kind) else {
            continue;
        };
        // The configured family may use any accepted alias (`embedding` /
        // `embeddings`, `fact-check` / `fact_check`, …).
        let Some(raw) = s
            .extra
            .iter()
            .find(|(k, _)| canonical_learned(k) == Some(canon))
            .map(|(_, v)| v)
        else {
            continue;
        };
        let items: Vec<serde_json::Value> = if raw.is_array() {
            raw.as_array().cloned().unwrap_or_default()
        } else if raw.is_object() {
            vec![raw.clone()]
        } else {
            continue;
        };
        for item in items {
            let Some(obj) = item.as_object() else {
                continue;
            };
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed")
                .to_string();
            // `model: <catalog_key>` selects the catalogue dim for this signal;
            // validate() guarantees the key resolves.
            let catalog_dim = obj
                .get("model")
                .and_then(|v| v.as_str())
                .and_then(|k| doc.global.model_catalog.get(k.trim()))
                .and_then(|e| e.dim)
                .or(fallback_dim);
            let dim = ml::dim_from(obj, catalog_dim);
            let (matched, confidence) = if canon == "embedding" {
                let threshold = obj
                    .get("threshold")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.75) as f32;
                let candidates: Vec<String> = obj
                    .get("candidates")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let agg = obj
                    .get("aggregation_method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("mean");
                ml::embed_match(text, &candidates, threshold, agg, dim)
            } else {
                ml::classify_match(text, obj, dim)
            };
            hits.push(SignalHit {
                kind: kind.clone(),
                name,
                matched,
                confidence,
            });
        }
    }
    Ok(hits)
}

fn referenced(decisions: &[aria_router_config::DecisionCfg]) -> Vec<String> {
    let mut kinds = vec![];
    for d in decisions {
        collect_kinds(&d.rules, &mut kinds);
    }
    kinds.sort();
    kinds.dedup();
    kinds
}

fn collect_kinds(node: &aria_router_config::RuleNode, out: &mut Vec<String>) {
    for c in &node.conditions {
        out.push(c.kind.clone());
    }
    if let Some(inner) = &node.not {
        collect_kinds(inner, out);
    }
}

fn eval_keywords(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    let lower = prompt.to_ascii_lowercase();
    s.keywords
        .iter()
        .map(|k| {
            let hits: Vec<bool> = k
                .keywords
                .iter()
                .map(|w| lower.contains(&w.to_ascii_lowercase()))
                .collect();
            let matched = match k.operator.to_ascii_uppercase().as_str() {
                "AND" => !hits.is_empty() && hits.iter().all(|x| *x),
                "NOR" => hits.iter().all(|x| !*x),
                _ => hits.iter().any(|x| *x),
            };
            SignalHit {
                kind: "keyword".into(),
                name: k.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_language(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    s.language
        .iter()
        .map(|n| {
            let want = n.extra.get("code").and_then(|v| v.as_str()).unwrap_or("en");
            let matched = detect_lang(prompt) == want;
            SignalHit {
                kind: "language".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn detect_lang(prompt: &str) -> &'static str {
    let cjk = prompt
        .chars()
        .filter(|c| {
            let u = *c as u32;
            (0x4E00..=0x9FFF).contains(&u) || (0x3040..=0x30FF).contains(&u)
        })
        .count();
    if cjk > prompt.chars().count() / 8 && cjk > 0 {
        "zh"
    } else {
        "en"
    }
}

fn eval_context(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    let tokens = (prompt.chars().count().div_ceil(4)) as u32;
    s.context
        .iter()
        .map(|c| {
            let ge = c.min_tokens.unwrap_or(0);
            let le = c.max_tokens.unwrap_or(u32::MAX);
            let matched = tokens >= ge && tokens <= le;
            SignalHit {
                kind: "context".into(),
                name: c.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_authz(s: &Signals, metadata: &HashMap<String, String>) -> Vec<SignalHit> {
    let role = metadata.get("role").cloned().unwrap_or_default();
    s.authz
        .iter()
        .map(|n| {
            let want = n.extra.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let matched = !want.is_empty() && role == want;
            SignalHit {
                kind: "authz".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_conversation(s: &Signals, req: &ChatRequest) -> Vec<SignalHit> {
    s.conversation
        .iter()
        .map(|c| {
            let min = c.min_messages.unwrap_or(1);
            let matched = req.messages.len() >= min;
            SignalHit {
                kind: "conversation".into(),
                name: c.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_metadata(s: &Signals, metadata: &HashMap<String, String>) -> Vec<SignalHit> {
    s.metadata
        .iter()
        .map(|m| {
            let matched = metadata.get(&m.key).is_some_and(|v| v == &m.value);
            SignalHit {
                kind: "metadata".into(),
                name: m.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_event(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    s.event
        .iter()
        .map(|n| {
            let needle = n
                .extra
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or(&n.name);
            let matched = prompt.contains(needle);
            SignalHit {
                kind: "event".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_structure(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    let qmarks = prompt.matches('?').count() + prompt.matches('？').count();
    s.structure
        .iter()
        .map(|n| {
            let min_q = n
                .extra
                .get("min_questions")
                .and_then(|v| v.as_u64())
                .unwrap_or(2);
            let matched = qmarks as u64 >= min_q;
            SignalHit {
                kind: "structure".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_config::RouterDocument;

    #[test]
    fn keyword_or() {
        let raw = include_str!("../../config/examples/semantic-tiny.yaml");
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let req = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: serde_json::Value::String("please explain rust".into()),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let set = extract(&doc, recipe, &req, &HashMap::new()).unwrap();
        assert!(set.matched("keyword", "needs_explain"));
    }

    #[test]
    fn keyword_uses_last_user_turn_only() {
        let raw = include_str!("../../config/examples/semantic-gateway.yaml");
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let req = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: serde_json::Value::String("please explain rust".into()),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: serde_json::Value::String("Rust is…".into()),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: serde_json::Value::String("hi".into()),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let set = extract(&doc, recipe, &req, &HashMap::new()).unwrap();
        assert!(
            !set.matched("keyword", "needs_explain"),
            "prior explain must not match after a later hi"
        );
        assert!(set.matched("conversation", "multi_turn"));
    }

    #[test]
    #[cfg(not(feature = "ml"))]
    fn learned_referenced_is_unsupported() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        keywords: []
        classifier: { name: intent }
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: classifier
                name: intent
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let req = ChatRequest {
            model: "auto".into(),
            messages: vec![],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let err = extract(&doc, recipe, &req, &HashMap::new()).unwrap_err();
        assert!(matches!(err, RouterError::Unsupported(_)));
    }

    /// Plural learned family referenced by a decision: the `ml`-off gate must
    /// reject it instead of silently producing no hit (B6).
    #[test]
    #[cfg(not(feature = "ml"))]
    fn learned_plural_referenced_is_unsupported() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        embeddings:
          - name: intent
            candidates: ["a"]
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: embeddings
                name: intent
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let err = extract(&doc, recipe, &req_with("hello"), &HashMap::new()).unwrap_err();
        assert!(matches!(err, RouterError::Unsupported(_)), "{err}");
        assert!(err.to_string().contains("embeddings"), "{err}");
    }

    /// Plural conditions resolve to the plural signal family and keep the original
    /// spelling on the hit so the condition still matches (B6).
    #[test]
    #[cfg(feature = "ml")]
    fn learned_plural_condition_hits_with_referenced_kind() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
global:
  model_catalog:
    embed_hash:
      dim: 16
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        embeddings:
          - name: tech
            model: embed_hash
            threshold: 0.05
            candidates: ["installation guide", "troubleshooting"]
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: embeddings
                name: tech
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let set = extract(
            &doc,
            recipe,
            &req_with("need installation guide"),
            &HashMap::new(),
        )
        .unwrap();
        assert!(
            set.matched("embeddings", "tech"),
            "hit kind must keep the YAML spelling"
        );
        assert!(set.get("embeddings", "tech").is_some());
    }

    /// `weight_path` in a learned signal means ONNX weights this build never loads,
    /// so the runtime gate must fail loudly even for documents built in memory (B5).
    #[test]
    fn learned_weight_path_is_unsupported_at_runtime() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        embeddings:
          - name: intent
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: embedding
                name: intent
          modelRefs: [{model: local/general}]
"#;
        let with_extra = |key: &str, value: serde_json::Value| {
            let mut doc = RouterDocument::from_yaml_str(raw).unwrap();
            doc.recipes[0]
                .routing
                .as_mut()
                .unwrap()
                .signals
                .extra
                .insert(key.into(), value);
            let recipe = doc.recipe("mom").unwrap();
            extract(&doc, recipe, &req_with("hi"), &HashMap::new())
        };

        // Array and mapping forms both trip the guard.
        for value in [
            serde_json::json!([{"name": "intent", "weight_path": "/tmp/e.onnx"}]),
            serde_json::json!({"name": "intent", "weight_path": "/tmp/e.onnx"}),
        ] {
            let err = with_extra("embeddings", value).unwrap_err();
            assert!(matches!(err, RouterError::Unsupported(_)), "{err}");
            assert!(err.to_string().contains("weight_path"), "{err}");
        }

        // Non-learned keys, scalar values and non-object items are tolerated.
        for (key, value) in [
            ("custom", serde_json::json!([{"weight_path": "/tmp/x"}])),
            ("embeddings", serde_json::json!(3)),
            ("embeddings", serde_json::json!([3])),
        ] {
            let _ = with_extra(key, value);
        }
    }

    /// `eval_learned` visits only referenced families and tolerates every declared
    /// shape (array / mapping / scalar / missing / non-object items).
    #[test]
    #[cfg(feature = "ml")]
    fn eval_learned_tolerates_mixed_and_missing_declarations() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        keywords:
          - name: hi
            keywords: [hello]
        embeddings:
          name: emb
          threshold: 0.05
          candidates: ["hello"]
        jailbreaks: "1"
        domains:
          - 3
          - name: d
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: hi
              - type: embeddings
                name: emb
              - type: jailbreak
                name: j
              - type: domain
                name: d
              - type: reask
                name: undeclared
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let set = extract(&doc, recipe, &req_with("hello"), &HashMap::new()).unwrap();
        assert!(set.matched("keyword", "hi"));
        // Mapping form is processed.
        assert!(set.get("embeddings", "emb").is_some());
        // Scalar family and non-object array items are skipped.
        assert!(set.get("jailbreak", "j").is_none());
        assert!(set.get("domain", "d").is_some());
        // Referenced but undeclared family produces nothing.
        assert!(set.get("reask", "undeclared").is_none());
    }

    #[test]
    #[cfg(feature = "ml")]
    fn learned_embedding_with_ml() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        embeddings:
          - name: tech
            threshold: 0.15
            candidates: ["installation guide", "troubleshooting"]
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: embedding
                name: tech
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let req = ChatRequest {
            model: "auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: serde_json::json!("need installation guide help"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let set = extract(&doc, recipe, &req, &HashMap::new()).unwrap();
        assert!(set.matched("embedding", "tech") || set.get("embedding", "tech").is_some());
    }

    #[test]
    #[cfg(feature = "ml")]
    fn learned_embedding_honours_dim_and_aggregation() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        embeddings:
          - name: tech
            threshold: 0.15
            dim: 32
            aggregation_method: max
            candidates: ["installation guide", "troubleshooting"]
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: embedding
                name: tech
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let set = extract(
            &doc,
            recipe,
            &req_with("need installation guide help"),
            &HashMap::new(),
        )
        .unwrap();
        let hit = set.get("embedding", "tech").expect("embedding hit");
        assert!(hit.matched, "confidence={}", hit.confidence);
        assert!(hit.confidence > 0.15, "confidence={}", hit.confidence);
    }

    fn req_with(text: &str) -> ChatRequest {
        ChatRequest {
            model: "auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: serde_json::Value::String(text.into()),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        }
    }

    const ALL_SIGNALS_YAML: &str = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        keywords:
          - name: needs_explain
            operator: AND
            keywords: [explain, rust]
          - name: none_hit
            operator: NOR
            keywords: [zzz]
        language:
          - name: zh
            code: zh
          - name: en
            code: en
        context:
          - name: tiny
            max_tokens: 1000
        authz:
          - name: admin
            role: admin
          - name: no_role
        conversation:
          - name: multi_turn
            min_messages: 2
        metadata:
          - name: tenant_a
            key: tenant
            value: a
        event:
          - name: build_failed
            code: BUILD_FAILED
          - name: bare_event
        structure:
          - name: many_questions
            min_questions: 2
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
              - type: language
                name: zh
              - type: context
                name: tiny
              - type: authz
                name: admin
              - type: conversation
                name: multi_turn
              - type: metadata
                name: tenant_a
              - type: event
                name: build_failed
              - type: structure
                name: many_questions
          modelRefs: [{model: local/general}]
"#;

    #[test]
    fn extracts_every_heuristic_signal_family() {
        let doc = RouterDocument::from_yaml_str(ALL_SIGNALS_YAML).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let mut md = HashMap::new();
        md.insert("role".to_string(), "admin".to_string());
        md.insert("tenant".to_string(), "a".to_string());

        // Chinese + two questions + the BUILD_FAILED event code.
        let req = req_with("请解释 explain rust ownership 所有权是什么意思？为什么？ BUILD_FAILED");
        let set = extract(&doc, recipe, &req, &md).unwrap();
        assert!(set.matched("keyword", "needs_explain"));
        assert!(set.matched("language", "zh"));
        assert!(!set.matched("language", "en"));
        assert!(set.matched("context", "tiny"));
        assert!(set.matched("authz", "admin"));
        // No `role` configured on this signal → never matches.
        assert!(!set.matched("authz", "no_role"));
        assert!(!set.matched("conversation", "multi_turn"));
        assert!(set.matched("metadata", "tenant_a"));
        assert!(set.matched("event", "build_failed"));
        assert!(set.matched("structure", "many_questions"));

        // Missing event code falls back to the signal name.
        let set = extract(&doc, recipe, &req_with("bare_event here"), &HashMap::new()).unwrap();
        assert!(set.matched("event", "bare_event"));
        assert!(!set.matched("event", "build_failed"));
        assert!(!set.matched("authz", "admin"));
        assert!(!set.matched("metadata", "tenant_a"));
        assert!(!set.matched("structure", "many_questions"));
        // `NOR` keyword signal matches because none of its words appear.
        assert!(set.matched("keyword", "none_hit"));
    }

    #[test]
    fn detect_lang_switches_on_cjk_density() {
        assert_eq!(detect_lang("plain english sentence"), "en");
        assert_eq!(detect_lang("这是一个中文句子"), "zh");
        assert_eq!(detect_lang(""), "en");
    }

    #[test]
    fn recipe_without_routing_yields_empty_set() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: agent
    recipe: bare
recipes:
  - name: bare
    router: agent
    agent:
      max_turns: 1
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("bare").unwrap();
        let set = extract(&doc, recipe, &req_with("hi"), &HashMap::new()).unwrap();
        assert!(set.hits.is_empty());
    }

    #[cfg(feature = "ml")]
    #[test]
    fn learned_signal_yaml_shapes_under_ml() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        fact-check:
          - name: fc
            description: "check the facts"
        jailbreak: {name: single, description: "jailbreak prompt"}
        domain: "not-a-list"
        pii:
          - "just a string"
          - name: pii_names
            pii_types_allowed: ["name", "address"]
          - name: pii_empty
        user-feedback:
          - name: uf
            examples: [{"text": "too slow"}, "not helpful"]
        complexity:
          - name: hard
            hard: ["prove the theorem"]
            easy: ["say hi"]
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: fact-check
                name: fc
              - type: jailbreak
                name: single
              - type: domain
                name: whatever
              - type: pii
                name: pii_names
              - type: user-feedback
                name: uf
              - type: complexity
                name: hard
              - type: reask
                name: absent
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let set = extract(
            &doc,
            recipe,
            &req_with("please check the facts and prove the theorem"),
            &HashMap::new(),
        )
        .unwrap();
        // Every configured learned signal is emitted (matched or not).
        assert!(set.get("fact-check", "fc").is_some());
        assert!(set.get("jailbreak", "single").is_some());
        assert!(set.get("pii", "pii_names").is_some());
        assert!(set.get("pii", "pii_empty").is_some());
        assert!(set.get("user-feedback", "uf").is_some());
        assert!(set.get("complexity", "hard").is_some());
        // Scalar `domain` and unconfigured `reask` produce no hits.
        assert!(set.get("domain", "whatever").is_none());
        assert!(set.get("reask", "absent").is_none());
    }

    #[test]
    fn nested_not_rules_contribute_their_signal_kinds() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      signals:
        keywords:
          - name: needs_explain
            keywords: [explain]
      decisions:
        - name: d
          rules:
            operator: AND
            conditions: []
            not:
              operator: AND
              conditions:
                - type: keyword
                  name: needs_explain
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let set = extract(&doc, recipe, &req_with("please explain"), &HashMap::new()).unwrap();
        assert!(set.matched("keyword", "needs_explain"));
    }
}
