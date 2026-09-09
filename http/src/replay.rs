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
