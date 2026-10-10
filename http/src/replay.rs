//! Thickened router replay log + optional JSONL persist.

use aria_router_config::{RetentionDirective, RouterReplayCfg};
use aria_router_core::RouteDecision;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// Execution-evidence fields. Populated where the data is
    /// available; `None` means "not recorded this turn" rather than an error.
    /// Task category chosen by the decision model (e.g. `draft` / `review`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_category: Option<String>,
    /// Selected model group / tier (`small` / `mid` / `large` / `unknown`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_group: Option<String>,
    /// Tool outcomes captured during an agent turn (JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_outcomes: Option<Value>,
    /// Accepted (final) output preview for the routed request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_output: Option<String>,
    /// Stage competence score (0..1) recorded for this decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_competence: Option<f32>,
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
            ..Default::default()
        }
    }

    #[test]
    fn evidence_fields_roundtrip() {
        let mut ev_rec = rec("d-evidence");
        ev_rec.task_category = Some("draft".into());
        ev_rec.selected_group = Some("small".into());
        ev_rec.tool_outcomes =
            Some(serde_json::json!([{"tool": "list_eligible_models", "ok": true}]));
        ev_rec.accepted_output = Some("final answer".into());
        ev_rec.stage_competence = Some(0.87);
        let line = serde_json::to_string(&ev_rec).unwrap();
        // Empty Option fields are omitted by skip_serializing_if.
        assert!(line.contains("task_category"));
        assert!(line.contains("stage_competence"));
        let parsed: ReplayRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.task_category.as_deref(), Some("draft"));
        assert_eq!(parsed.selected_group.as_deref(), Some("small"));
        assert_eq!(parsed.accepted_output.as_deref(), Some("final answer"));
        assert_eq!(parsed.stage_competence, Some(0.87));
        assert_eq!(
            parsed.tool_outcomes.as_ref().unwrap()[0]["tool"],
            "list_eligible_models"
        );

        // Record without evidence omits the keys entirely.
        let rec2 = rec("d-plain");
        let line2 = serde_json::to_string(&rec2).unwrap();
        assert!(!line2.contains("task_category"));
    }

    #[test]
    fn selected_group_derives_tier() {
        assert_eq!(
            aria_router_core::tier_of_model("ariacompute/ariamodel-small"),
            "small"
        );
        assert_eq!(
            aria_router_core::tier_of_model("ariacompute/ariamodel-mid"),
            "mid"
        );
        assert_eq!(
            aria_router_core::tier_of_model("ariacompute/ariamodel-large"),
            "large"
        );
        assert_eq!(aria_router_core::tier_of_model("local/general"), "unknown");
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
