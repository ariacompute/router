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
