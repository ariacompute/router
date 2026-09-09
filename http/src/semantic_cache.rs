//! Category / decision-aware semantic response cache (hash embedding similarity).

use aria_router_config::SemanticCacheCfg;
use serde_json::Value;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

#[derive(Debug, Clone)]
struct Entry {
    vec: Vec<f32>,
    model: String,
    decision: String,
    body: Value,
    ttl_left: u32,
}

#[derive(Debug, Default)]
pub struct SemanticCache {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    entries: Vec<Entry>,
    order: VecDeque<usize>,
}

impl SemanticCache {
    pub fn lookup(
        &self,
        text: &str,
        model: &str,
        decision: &str,
        cfg: &SemanticCacheCfg,
    ) -> Option<Value> {
        if !cfg.enabled {
            return None;
        }
        let q = hash_embed(text, 64);
        let g = self.inner.lock().unwrap();
        let mut best: Option<(f32, &Entry)> = None;
        for e in &g.entries {
            if e.model != model {
                continue;
            }
            // Prefer same decision; still allow cross-decision if similar enough.
            let bonus = if e.decision == decision { 0.02 } else { 0.0 };
            let sim = cosine(&q, &e.vec) + bonus;
            if sim >= cfg.similarity_threshold
                && best.as_ref().is_none_or(|(s, _)| sim > *s)
            {
                best = Some((sim, e));
            }
        }
        best.map(|(_, e)| e.body.clone())
    }

    pub fn store(
        &self,
        text: &str,
        model: &str,
        decision: &str,
        body: Value,
        ttl_turns: u32,
        cfg: &SemanticCacheCfg,
    ) {
        if !cfg.enabled {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        let idx = g.entries.len();
        g.entries.push(Entry {
            vec: hash_embed(text, 64),
            model: model.to_string(),
            decision: decision.to_string(),
            body,
            ttl_left: ttl_turns.max(1),
        });
        g.order.push_back(idx);
        while g.entries.len() > cfg.max_entries {
            if let Some(old) = g.order.pop_front() {
                if old < g.entries.len() {
                    g.entries.remove(old);
                    // Rebuild order indices — keep simple by clearing order on overflow trim.
                    g.order.clear();
                    for i in 0..g.entries.len() {
                        g.order.push_back(i);
                    }
                    break;
                }
            } else {
                break;
            }
        }
    }

    pub fn tick_all(&self) {
        let mut g = self.inner.lock().unwrap();
        g.entries.retain_mut(|e| {
            if e.ttl_left <= 1 {
                false
            } else {
                e.ttl_left -= 1;
                true
            }
        });
        g.order.clear();
        for i in 0..g.entries.len() {
            g.order.push_back(i);
        }
    }
}

fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    for tok in text.split_whitespace() {
        let t = tok.to_ascii_lowercase();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        t.hash(&mut h);
        let idx = (h.finish() as usize) % dim;
        v[idx] += 1.0;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for x in &mut v {
        *x /= norm;
    }
    v
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let d = na.sqrt() * nb.sqrt();
    if d < 1e-9 {
        0.0
    } else {
        (dot / d).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(threshold: f32, max_entries: usize) -> SemanticCacheCfg {
        SemanticCacheCfg {
            enabled: true,
            similarity_threshold: threshold,
            max_entries,
            ttl_turns: 4,
        }
    }

    #[test]
    fn hit_miss_model_isolation_and_disabled() {
        let cache = SemanticCache::default();
        let c = cfg(0.5, 64);
        cache.store(
            "explain rust ownership",
            "m1",
            "d1",
            json!({"cached": true}),
            4,
            &c,
        );
        assert_eq!(
            cache
                .lookup("explain rust ownership", "m1", "d1", &c)
                .unwrap()["cached"],
            true
        );
        assert!(cache.lookup("explain rust ownership", "m2", "d1", &c).is_none());
        assert!(cache
            .lookup("totally unrelated zzzyx", "m1", "d1", &cfg(0.99, 64))
            .is_none());

        let off = SemanticCacheCfg {
            enabled: false,
            ..cfg(0.5, 64)
        };
        assert!(cache
            .lookup("explain rust ownership", "m1", "d1", &off)
            .is_none());
        cache.store("x", "m1", "d1", json!(1), 1, &off);
    }

    #[test]
    fn ttl_expiry_and_max_entries() {
        let cache = SemanticCache::default();
        let c = cfg(0.5, 2);
        cache.store("alpha one", "m", "d", json!(1), 2, &c);
        cache.tick_all();
        assert!(cache.lookup("alpha one", "m", "d", &c).is_some());
        cache.tick_all();
        assert!(cache.lookup("alpha one", "m", "d", &c).is_none());

        cache.store("zzzz unique alpha", "m", "d", json!("a"), 4, &c);
        cache.store("yyyy unique beta", "m", "d", json!("b"), 4, &c);
        cache.store("xxxx unique gamma", "m", "d", json!("c"), 4, &c);
        // max_entries=2 → oldest trimmed; exact key for oldest misses.
        assert!(cache.lookup("zzzz unique alpha", "m", "d", &cfg(0.99, 2)).is_none());
        assert!(cache.lookup("xxxx unique gamma", "m", "d", &cfg(0.99, 2)).is_some());
    }

    #[test]
    fn hash_embed_cosine_basics() {
        let a = hash_embed("hello world", 32);
        let b = hash_embed("hello world", 32);
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-5);
        assert_eq!(cosine(&[], &[]), 0.0);
    }
}
