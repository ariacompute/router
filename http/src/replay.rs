//! Thickened router replay log + optional JSONL persist.

use crate::policy::StageEvidence;
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
    /// Entrypoint that produced the decision (cross-entrypoint evidence guard).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Plan / rubric versions and problem continuation of the scored stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<f32>,
    /// `ok` | `error` | `interrupted`: explicit execution outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_outcome: Option<String>,
    /// Classified cross-request boundary for this record (C1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redecision_boundary: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReplayLog {
    inner: Mutex<VecDeque<ReplayRecord>>,
    persist: Mutex<Option<PathBuf>>,
}

impl ReplayLog {
    pub fn configure(&self, cfg: &RouterReplayCfg) {
        let path = cfg.persist_path.as_ref().map(PathBuf::from);
        if let Some(p) = &path {
            if let Ok(body) = std::fs::read_to_string(p) {
                let mut g = self.inner.lock().unwrap();
                for line in body.lines() {
                    // Old JSONL lacks the evidence fields: serde defaults keep
                    // those records readable. Last line wins for a repeated id.
                    if let Ok(rec) = serde_json::from_str::<ReplayRecord>(line) {
                        g.retain(|r| r.id != rec.id);
                        g.push_back(rec);
                    }
                }
                while g.len() > cfg.max_items {
                    g.pop_front();
                }
            }
        }
        *self.persist.lock().unwrap() = path;
    }

    pub fn push(&self, mut rec: ReplayRecord, cfg: &RouterReplayCfg) {
        if !cfg.enabled {
            return;
        }
        if rec.id.is_empty() {
            rec.id = format!("r{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        }
        self.append_jsonl(std::slice::from_ref(&rec));
        let mut g = self.inner.lock().unwrap();
        g.push_back(rec);
        while g.len() > cfg.max_items {
            g.pop_front();
        }
    }

    fn append_jsonl(&self, recs: &[ReplayRecord]) {
        let Some(path) = self.persist.lock().unwrap().clone() else {
            return;
        };
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            for rec in recs {
                // Serialization of a replay record cannot fail; an empty fallback
                // line keeps the write non-branching.
                let line = serde_json::to_string(rec).unwrap_or_default();
                let _ = writeln!(f, "{line}");
            }
        }
    }

    /// Attach execution evidence to every record of one request (the plain
    /// record plus the `:route` rich record). Returns false when the request
    /// produced no records, e.g. replay was disabled.
    pub fn patch_evidence(&self, replay_id: &str, ev: &StageEvidence) -> bool {
        if replay_id.is_empty() {
            return false;
        }
        let prefix = format!("{replay_id}:");
        let mut changed = Vec::new();
        {
            let mut g = self.inner.lock().unwrap();
            for r in g.iter_mut() {
                if r.id != replay_id && !r.id.starts_with(&prefix) {
                    continue;
                }
                r.task_category = Some(ev.task_category.clone());
                r.stage_competence = ev.score;
                r.accepted_output = ev.accepted_output.clone();
                r.tool_outcomes = ev.tool_outcomes.clone();
                r.entrypoint = Some(ev.entrypoint.clone());
                r.plan_version = ev.plan_version.clone();
                r.rubric_version = ev.rubric_version.clone();
                r.continuation = ev.continuation;
                r.evidence_outcome = Some(ev.outcome.clone());
                changed.push(r.clone());
            }
        }
        if changed.is_empty() {
            return false;
        }
        // Append the enriched snapshot: JSONL stays append-only and replayable
        // (configure() applies last-write-wins by id).
        self.append_jsonl(&changed);
        true
    }

    pub fn set_boundary(&self, replay_id: &str, boundary: &str) {
        let prefix = format!("{replay_id}:");
        let mut g = self.inner.lock().unwrap();
        for r in g.iter_mut() {
            if r.id == replay_id || r.id.starts_with(&prefix) {
                r.redecision_boundary = Some(boundary.to_string());
            }
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

    fn evidence(id: &str) -> StageEvidence {
        StageEvidence {
            replay_id: id.into(),
            session: "s1".into(),
            entrypoint: "ep".into(),
            model: "local/small".into(),
            tier: "small".into(),
            task_category: "draft".into(),
            score: Some(0.42),
            plan_version: Some("plan-1".into()),
            rubric_version: Some("rubric-1".into()),
            continuation: Some(0.2),
            complete: true,
            ts_unix: 1.0,
            outcome: "ok".into(),
            accepted_output: Some("answer".into()),
            tool_outcomes: Some(serde_json::json!([{"tool": "echo", "ok": true}])),
        }
    }

    #[test]
    fn patch_evidence_enriches_all_records_of_one_request() {
        let log = ReplayLog::default();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 8,
            persist_path: None,
        };
        let mut plain = rec("plain");
        plain.id = "rp_1".into();
        let mut rich = rec("rich");
        rich.id = "rp_1:route".into();
        let mut unrelated = rec("other");
        unrelated.id = "rp_2".into();
        log.push(plain, &cfg);
        log.push(rich, &cfg);
        log.push(unrelated, &cfg);
        log.set_boundary("rp_1", "new-user-message");
        assert!(log.patch_evidence("rp_1", &evidence("rp_1")));
        assert!(!log.patch_evidence("", &evidence("rp_1")));
        assert!(!log.patch_evidence("missing", &evidence("rp_1")));
        let records = log.recent(8);
        let plain = records.iter().find(|r| r.id == "rp_1").unwrap();
        let rich = records.iter().find(|r| r.id == "rp_1:route").unwrap();
        let other = records.iter().find(|r| r.id == "rp_2").unwrap();
        for r in [plain, rich] {
            assert_eq!(r.task_category.as_deref(), Some("draft"));
            assert_eq!(r.stage_competence, Some(0.42));
            assert_eq!(r.accepted_output.as_deref(), Some("answer"));
            assert_eq!(r.entrypoint.as_deref(), Some("ep"));
            assert_eq!(r.plan_version.as_deref(), Some("plan-1"));
            assert_eq!(r.continuation, Some(0.2));
            assert_eq!(r.evidence_outcome.as_deref(), Some("ok"));
            assert_eq!(r.redecision_boundary.as_deref(), Some("new-user-message"));
            assert!(r.tool_outcomes.is_some());
        }
        assert!(other.task_category.is_none());
        assert!(other.stage_competence.is_none());
    }

    #[test]
    fn persisted_evidence_roundtrip_and_legacy_jsonl_compatibility() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.jsonl");
        // Legacy JSONL: no evidence fields at all.
        fs::write(&path, "{\"id\":\"old\",\"decision\":{\"model\":\"m\"}}\n").unwrap();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 8,
            persist_path: Some(path.display().to_string()),
        };
        let log = ReplayLog::default();
        log.configure(&cfg);
        assert!(log.get("old").is_some());
        let mut r = rec("new");
        r.id = "new".into();
        log.push(r, &cfg);
        log.patch_evidence("new", &evidence("new"));
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("stage_competence"));
        let reload = ReplayLog::default();
        reload.configure(&cfg);
        assert_eq!(reload.recent(8).len(), 2, "last-write-wins by id");
        let updated = reload.get("new").unwrap();
        assert_eq!(updated.stage_competence, Some(0.42));
        assert!(reload.get("old").unwrap().task_category.is_none());
    }

    #[test]
    fn configure_ignores_unreadable_or_malformed_persist_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.jsonl");
        fs::write(
            &path,
            "not json\n{\"id\":\"ok\",\"decision\":{\"model\":\"m\"}}\n",
        )
        .unwrap();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 1,
            persist_path: Some(path.display().to_string()),
        };
        let log = ReplayLog::default();
        log.configure(&cfg);
        assert_eq!(log.recent(4).len(), 1);
        let missing = RouterReplayCfg {
            persist_path: Some(dir.path().join("missing.jsonl").display().to_string()),
            ..cfg
        };
        let log2 = ReplayLog::default();
        log2.configure(&missing);
        assert_eq!(log2.recent(4).len(), 0);
    }

    #[test]
    fn persist_open_failure_keeps_in_memory_record() {
        let dir = tempfile::tempdir().unwrap();
        // A directory path cannot be opened for append: the write is skipped, but
        // the in-memory ring buffer still records the entry.
        let log = ReplayLog::default();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 4,
            persist_path: Some(dir.path().display().to_string()),
        };
        log.configure(&cfg);
        log.push(rec("persist-failed"), &cfg);
        let recent = log.recent(1);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].decision.decision, "persist-failed");
    }

    #[test]
    fn configure_caps_loaded_history_to_max_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let lines = (0..6)
            .map(|i| format!("{{\"id\":\"r{i}\",\"decision\":{{\"model\":\"m\"}}}}\n"))
            .collect::<String>();
        std::fs::write(&path, lines).unwrap();
        let cfg = RouterReplayCfg {
            enabled: true,
            max_items: 2,
            persist_path: Some(path.display().to_string()),
        };
        let log = ReplayLog::default();
        log.configure(&cfg);
        assert_eq!(log.recent(8).len(), 2);
        assert!(log.get("r4").is_some());
        assert!(log.get("r5").is_some());
        assert!(log.get("r0").is_none());
    }
}
