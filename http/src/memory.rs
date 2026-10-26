//! Process-in session memory for retention sticky models.

use aria_router_config::MemoryStoreCfg;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct SessionState {
    pub sticky_model: String,
    pub ttl_turns_left: u32,
    pub decision: String,
    /// Last user-message count observed for TTL ticking (ContextHold).
    pub last_user_turn: u64,
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

    /// Pin a sticky model. Same model re-pin keeps remaining TTL (no refresh);
    /// a different model starts a fresh `ttl` budget.
    pub fn pin(
        &self,
        session: &str,
        sticky_model: String,
        decision: String,
        ttl: u32,
        user_turn: u64,
        cfg: &MemoryStoreCfg,
    ) {
        if !cfg.enabled || session.is_empty() {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        if let Some(existing) = g.by_session.get_mut(session) {
            if existing.sticky_model == sticky_model {
                existing.decision = decision;
                if user_turn > existing.last_user_turn {
                    existing.last_user_turn = user_turn;
                }
                return;
            }
        }
        if !g.by_session.contains_key(session) {
            g.order.push_back(session.to_string());
        }
        g.by_session.insert(
            session.to_string(),
            SessionState {
                sticky_model,
                ttl_turns_left: ttl.max(1),
                decision,
                last_user_turn: user_turn,
            },
        );
        while g.by_session.len() > cfg.max_sessions {
            if let Some(old) = g.order.pop_front() {
                g.by_session.remove(&old);
            } else {
                break;
            }
        }
    }

    /// Decrement TTL only when the user-message count rises; remove when exhausted.
    /// Same-turn tool-call continuations (unchanged turn) do not tick.
    pub fn tick_if_new_turn(&self, session: &str, user_turn: u64) {
        let mut g = self.inner.lock().unwrap();
        let remove = if let Some(st) = g.by_session.get_mut(session) {
            if user_turn > st.last_user_turn {
                st.last_user_turn = user_turn;
                if st.ttl_turns_left <= 1 {
                    true
                } else {
                    st.ttl_turns_left -= 1;
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if remove {
            g.by_session.remove(session);
            g.order.retain(|s| s != session);
        }
    }

    /// Decrement TTL after a successful turn; remove when exhausted.
    ///
    /// Prefer [`tick_if_new_turn`] for ContextHold; this remains for direct unit tests.
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
            g.order.retain(|s| s != session);
        }
    }

    pub fn clear(&self, session: &str) {
        let mut g = self.inner.lock().unwrap();
        g.by_session.remove(session);
        g.order.retain(|s| s != session);
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
            last_user_turn: 0,
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

    #[test]
    fn tick_if_new_turn_same_turn_noop() {
        let mem = SessionMemory::default();
        mem.pin("s", "m1".into(), "d".into(), 3, 1, &cfg(8));
        mem.tick_if_new_turn("s", 1);
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 3);
        mem.tick_if_new_turn("s", 1);
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 3);
    }

    #[test]
    fn tick_if_new_turn_decrements_and_expires() {
        let mem = SessionMemory::default();
        mem.pin("s", "m1".into(), "d".into(), 2, 1, &cfg(8));
        mem.tick_if_new_turn("s", 2);
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 1);
        mem.tick_if_new_turn("s", 3);
        assert!(mem.get("s").is_none());
    }

    #[test]
    fn pin_same_model_does_not_reset_ttl() {
        let mem = SessionMemory::default();
        mem.pin("s", "m1".into(), "d1".into(), 4, 1, &cfg(8));
        mem.tick_if_new_turn("s", 2);
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 3);
        mem.pin("s", "m1".into(), "d2".into(), 4, 2, &cfg(8));
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 3);
        assert_eq!(mem.get("s").unwrap().decision, "d2");
    }

    #[test]
    fn pin_different_model_resets_ttl() {
        let mem = SessionMemory::default();
        mem.pin("s", "m1".into(), "d1".into(), 4, 1, &cfg(8));
        mem.tick_if_new_turn("s", 2);
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 3);
        mem.pin("s", "m2".into(), "d2".into(), 4, 2, &cfg(8));
        assert_eq!(mem.get("s").unwrap().sticky_model, "m2");
        assert_eq!(mem.get("s").unwrap().ttl_turns_left, 4);
    }

    #[test]
    fn pin_disabled_and_empty_session_noop() {
        let mem = SessionMemory::default();
        let disabled = MemoryStoreCfg {
            enabled: false,
            max_sessions: 8,
            default_ttl_turns: 2,
        };
        mem.pin("s", "m1".into(), "d".into(), 2, 1, &disabled);
        assert!(mem.get("s").is_none());
        mem.pin("", "m1".into(), "d".into(), 2, 1, &cfg(8));
        assert!(mem.get("").is_none());
    }

    #[test]
    fn pin_fifo_eviction_and_tick_missing_noop() {
        let mem = SessionMemory::default();
        mem.pin("a", "ma".into(), "d".into(), 2, 1, &cfg(2));
        mem.pin("b", "mb".into(), "d".into(), 2, 1, &cfg(2));
        mem.pin("c", "mc".into(), "d".into(), 2, 1, &cfg(2));
        assert!(mem.get("a").is_none());
        assert!(mem.get("b").is_some());
        assert!(mem.get("c").is_some());
        mem.tick_if_new_turn("missing", 9); // no-op
        mem.tick_if_new_turn("b", 0); // turn not increased → no-op
        assert_eq!(mem.get("b").unwrap().ttl_turns_left, 2);
    }

    #[test]
    fn pin_same_model_advances_last_user_turn() {
        let mem = SessionMemory::default();
        mem.pin("s", "m1".into(), "d1".into(), 3, 1, &cfg(8));
        mem.pin("s", "m1".into(), "d2".into(), 9, 5, &cfg(8));
        let st = mem.get("s").unwrap();
        assert_eq!(st.ttl_turns_left, 3); // not refreshed
        assert_eq!(st.last_user_turn, 5);
        assert_eq!(st.decision, "d2");
    }
}
