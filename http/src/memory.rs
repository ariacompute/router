//! Process-in session memory for retention sticky models.

use aria_router_config::MemoryStoreCfg;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct SessionState {
    pub sticky_model: String,
    pub ttl_turns_left: u32,
    pub decision: String,
}

#[derive(Debug, Default)]
pub struct SessionMemory {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    by_session: HashMap<String, SessionState>,
    order: VecDeque<String>,
}

impl SessionMemory {
    pub fn get(&self, session: &str) -> Option<SessionState> {
        self.inner.lock().unwrap().by_session.get(session).cloned()
    }

    pub fn put(&self, session: &str, state: SessionState, cfg: &MemoryStoreCfg) {
        if !cfg.enabled || session.is_empty() {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        if !g.by_session.contains_key(session) {
            g.order.push_back(session.to_string());
        }
        g.by_session.insert(session.to_string(), state);
        while g.by_session.len() > cfg.max_sessions {
            if let Some(old) = g.order.pop_front() {
                g.by_session.remove(&old);
            } else {
                break;
            }
        }
    }

    /// Decrement TTL after a successful turn; remove when exhausted.
    pub fn tick(&self, session: &str) {
        let mut g = self.inner.lock().unwrap();
        let remove = if let Some(st) = g.by_session.get_mut(session) {
            if st.ttl_turns_left <= 1 {
                true
            } else {
                st.ttl_turns_left -= 1;
                false
            }
        } else {
            false
        };
        if remove {
            g.by_session.remove(session);
        }
    }

    pub fn clear(&self, session: &str) {
        let mut g = self.inner.lock().unwrap();
        g.by_session.remove(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(max_sessions: usize) -> MemoryStoreCfg {
        MemoryStoreCfg {
            enabled: true,
            max_sessions,
            default_ttl_turns: 2,
        }
    }

    fn state(model: &str, ttl: u32) -> SessionState {
        SessionState {
            sticky_model: model.into(),
            ttl_turns_left: ttl,
            decision: "d".into(),
        }
    }

    #[test]
    fn put_get_and_disabled_noop() {
        let mem = SessionMemory::default();
        mem.put("s1", state("m1", 3), &cfg(8));
        assert_eq!(mem.get("s1").unwrap().sticky_model, "m1");

        let disabled = MemoryStoreCfg {
            enabled: false,
            max_sessions: 8,
            default_ttl_turns: 2,
        };
        mem.put("s2", state("m2", 1), &disabled);
        assert!(mem.get("s2").is_none());

        mem.put("", state("m3", 1), &cfg(8));
        assert!(mem.get("").is_none());
    }

    #[test]
    fn fifo_eviction_and_ttl_tick() {
        let mem = SessionMemory::default();
        mem.put("a", state("ma", 2), &cfg(2));
        mem.put("b", state("mb", 2), &cfg(2));
        mem.put("c", state("mc", 2), &cfg(2));
        assert!(mem.get("a").is_none());
        assert!(mem.get("b").is_some());
        assert!(mem.get("c").is_some());

        mem.put("ttl", state("mt", 1), &cfg(8));
        mem.tick("ttl");
        assert!(mem.get("ttl").is_none());

        mem.put("ttl2", state("mt", 2), &cfg(8));
        mem.tick("ttl2");
        assert_eq!(mem.get("ttl2").unwrap().ttl_turns_left, 1);
        mem.tick("ttl2");
        assert!(mem.get("ttl2").is_none());

        mem.put("x", state("mx", 5), &cfg(8));
        mem.clear("x");
        assert!(mem.get("x").is_none());
        mem.tick("missing"); // no-op
    }
}
