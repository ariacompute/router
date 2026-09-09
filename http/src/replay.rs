//! Thickened router replay log + optional JSONL persist.

use aria_router_config::{RetentionDirective, RouterReplayCfg};
use aria_router_core::RouteDecision;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRecord {
    pub id: String,
    pub decision: RouteDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals_summary: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projections: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emits: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<RetentionDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_preview: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReplayLog {
    inner: Mutex<VecDeque<ReplayRecord>>,
    persist: Mutex<Option<PathBuf>>,
}

impl ReplayLog {
    pub fn configure(&self, cfg: &RouterReplayCfg) {
        let path = cfg.persist_path.as_ref().map(PathBuf::from);
        *self.persist.lock().unwrap() = path;
    }

    pub fn push(&self, mut rec: ReplayRecord, cfg: &RouterReplayCfg) {
        if !cfg.enabled {
            return;
        }
        if rec.id.is_empty() {
            rec.id = format!("r{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        }
        if let Some(path) = self.persist.lock().unwrap().as_ref() {
            if let Ok(line) = serde_json::to_string(&rec) {
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
        let mut g = self.inner.lock().unwrap();
        g.push_back(rec);
        while g.len() > cfg.max_items {
            g.pop_front();
        }
    }

    pub fn recent(&self, n: usize) -> Vec<ReplayRecord> {
        let g = self.inner.lock().unwrap();
        g.iter().rev().take(n).cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<ReplayRecord> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_core::RouteDecision;
    use std::fs;
    use std::io::Read;

    fn rec(decision: &str) -> ReplayRecord {
        ReplayRecord {
            id: String::new(),
            decision: RouteDecision {
                model: "m".into(),
                decision: decision.into(),
                ..Default::default()
            },
            signals_summary: None,
            projections: None,
            emits: None,
            retention: None,
            session: Some("s1".into()),
            prompt_preview: Some("hi".into()),
        }
    }

    #[test]
    fn ring_buffer_ids_and_lookup() {
        let log = ReplayLog::default();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 2,
            persist_path: None,
        };
        log.push(rec("d1"), &cfg);
        log.push(rec("d2"), &cfg);
        log.push(rec("d3"), &cfg);
        let recent = log.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].decision.decision, "d3");
        assert_eq!(recent[1].decision.decision, "d2");
        assert!(!recent[0].id.is_empty());
        assert_ne!(recent[0].id, recent[1].id);
        assert!(log.get(&recent[0].id).is_some());
        assert!(log.get("missing").is_none());

        let off = RouterReplayCfg {
            enabled: false,
            max_items: 10,
            persist_path: None,
        };
        let before = log.recent(10).len();
        log.push(rec("noop"), &off);
        assert_eq!(log.recent(10).len(), before);
    }

    #[test]
    fn persist_jsonl_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.jsonl");
        let log = ReplayLog::default();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 8,
            persist_path: Some(path.display().to_string()),
        };
        log.configure(&cfg);
        log.push(rec("persist-me"), &cfg);
        let mut f = fs::File::open(&path).unwrap();
        let mut buf = String::new();
        f.read_to_string(&mut buf).unwrap();
        let line = buf.lines().next().unwrap();
        let parsed: ReplayRecord = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.decision.decision, "persist-me");
        assert_eq!(parsed.session.as_deref(), Some("s1"));
    }
}
