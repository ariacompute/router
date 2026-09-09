//! Elo ratings table + selection helpers.

use std::collections::HashMap;
use std::sync::Mutex;

/// In-process Elo ratings (default 1000). Shared across requests in one serve.
#[derive(Debug, Default)]
pub struct EloTable {
    inner: Mutex<HashMap<String, f32>>,
}

impl EloTable {
    pub fn rating(&self, model: &str) -> f32 {
        self.inner
            .lock()
            .unwrap()
            .get(model)
            .copied()
            .unwrap_or(1000.0)
    }

    pub fn set(&self, model: &str, rating: f32) {
        self.inner.lock().unwrap().insert(model.to_string(), rating);
    }

    /// Update ratings after an outcome: winner gains, loser loses (K=24).
    pub fn update_pair(&self, winner: &str, loser: &str) {
        let mut g = self.inner.lock().unwrap();
        let rw = *g.get(winner).unwrap_or(&1000.0);
        let rl = *g.get(loser).unwrap_or(&1000.0);
        let ew = 1.0 / (1.0 + 10f32.powf((rl - rw) / 400.0));
        let el = 1.0 - ew;
        let k = 24.0;
        g.insert(winner.to_string(), rw + k * (1.0 - ew));
        g.insert(loser.to_string(), rl + k * (0.0 - el));
    }

    /// Prefer lower latency as a soft win signal for `model`.
    pub fn observe_latency(&self, model: &str, latency_ms: f32, peers: &[String]) {
        if peers.is_empty() {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        let r = *g.get(model).unwrap_or(&1000.0);
        // Faster than 100ms baseline → slight rating bump.
        let score = if latency_ms < 100.0 {
            1.0
        } else if latency_ms < 500.0 {
            0.5
        } else {
            0.0
        };
        let expected = 0.5;
        g.insert(model.to_string(), r + 16.0 * (score - expected));
        let _ = peers;
    }
}

/// Global process table used by `elo` / `ratings` algorithms.
pub fn global_elo() -> &'static EloTable {
    use std::sync::OnceLock;
    static TABLE: OnceLock<EloTable> = OnceLock::new();
    TABLE.get_or_init(EloTable::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_pair_and_latency() {
        let t = EloTable::default();
        assert!((t.rating("a") - 1000.0).abs() < 1e-3);
        t.update_pair("a", "b");
        assert!(t.rating("a") > t.rating("b"));
        t.set("c", 1000.0);
        t.observe_latency("c", 50.0, &["peer".into()]);
        let fast = t.rating("c");
        assert!(fast > 1000.0);
        t.observe_latency("c", 600.0, &["peer".into()]);
        assert!(t.rating("c") < fast);
        t.observe_latency("d", 10.0, &[]); // no-op without peers
        assert!((t.rating("d") - 1000.0).abs() < 1e-3);
        let _ = global_elo().rating("x");
    }
}
