//! Runtime state for decision feedback (C3/C4/C5/C6/C7).
//!
//! Everything here is process-local and explicitly keyed by session / key id.
//! No value is written to logs, and no state can widen the hard-filtered
//! eligible pool: protection, trust tiers and evidence are Preference-layer
//! signals consumed only *after* Location / auth / modality pruning.

use aria_router_config::TrustCfg;
use aria_router_core::RouterError;
use aria_router_decision::{ProtectionEvidence, ProtectionState};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cost::CostEvent;

pub fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Truncate untrusted text to a preview, matching the existing 200-char
/// `prompt_preview` convention. Never stores full request / response bodies.
pub fn preview(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// One execution observation produced by the production chat path (C6).
///
/// `score = None` means "unscored", never zero. `outcome` is explicit so a
/// failed tool call or an interrupted SSE stream is never mistaken for a
/// successful, accepted output.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StageEvidence {
    pub replay_id: String,
    pub session: String,
    pub entrypoint: String,
    pub model: String,
    pub tier: String,
    pub task_category: String,
    pub score: Option<f32>,
    pub plan_version: Option<String>,
    pub rubric_version: Option<String>,
    pub continuation: Option<f32>,
    pub complete: bool,
    pub ts_unix: f64,
    pub outcome: String,
    pub accepted_output: Option<String>,
    pub tool_outcomes: Option<Value>,
}

impl StageEvidence {
    pub fn age_secs(&self, now: f64) -> u64 {
        (now - self.ts_unix).max(0.0) as u64
    }

    pub fn to_protection_evidence(&self, now: f64) -> ProtectionEvidence {
        ProtectionEvidence {
            key: self.replay_id.clone(),
            score: self.score,
            category: self.task_category.clone(),
            plan_version: self.plan_version.clone().unwrap_or_default(),
            rubric_version: self.rubric_version.clone().unwrap_or_default(),
            continuation: self.continuation.unwrap_or(1.0),
            complete: self.complete && self.outcome == "ok",
            age_secs: self.age_secs(now),
        }
    }
}

/// Per-request evidence input captured before forwarding (C6). Values come
/// from truncated response previews and explicit client metadata only — never
/// from credentials or the full request body.
#[derive(Debug, Clone, Default)]
pub struct EvidenceSeed {
    pub replay_id: String,
    pub session: String,
    pub entrypoint: String,
    pub model: String,
    pub tier: String,
    pub task_category: String,
    pub score: Option<f32>,
    pub plan_version: Option<String>,
    pub rubric_version: Option<String>,
    pub continuation: Option<f32>,
    pub tools_declared: bool,
    pub tool_outcomes: Option<Value>,
}

impl EvidenceSeed {
    /// Partial grading (missing score / plan / rubric / category) is never a
    /// zero score: it is incomplete evidence and cannot trigger protection.
    pub fn complete(&self) -> bool {
        self.score.is_some()
            && self.plan_version.is_some()
            && self.rubric_version.is_some()
            && !self.task_category.is_empty()
    }

    pub fn finish(
        &self,
        outcome: &str,
        accepted_output: Option<String>,
        tool_outcomes: Option<Value>,
        now: f64,
    ) -> StageEvidence {
        let tools = tool_outcomes
            .or_else(|| self.tool_outcomes.clone())
            .or_else(|| {
                self.tools_declared
                    .then(|| json!([{"status": "no-outcome"}]))
            });
        StageEvidence {
            replay_id: self.replay_id.clone(),
            session: self.session.clone(),
            entrypoint: self.entrypoint.clone(),
            model: self.model.clone(),
            tier: self.tier.clone(),
            task_category: self.task_category.clone(),
            score: self.score,
            plan_version: self.plan_version.clone(),
            rubric_version: self.rubric_version.clone(),
            continuation: self.continuation,
            complete: self.complete(),
            ts_unix: now,
            outcome: outcome.to_string(),
            accepted_output,
            tool_outcomes: tools,
        }
    }
}

/// Trust-tier draft. A draft never changes online routing; only
/// `publish_trust` creates a publication, and publication is deliberately
/// separated from `PUT /v1/router/config` (which still applies immediately).
#[derive(Debug, Clone, Serialize)]
pub struct TrustDraft {
    pub id: String,
    pub created_at: f64,
    pub entrypoint: String,
    pub tier: String,
    pub action: String,
    pub confidence: f64,
    pub sample_size: usize,
    pub reasons: Vec<String>,
    pub published: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustPublication {
    pub id: String,
    pub draft_id: String,
    pub version: u64,
    pub tier: String,
    pub action: String,
    pub confidence: f64,
    pub published_at: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustOutcome {
    pub status: String,
    pub usable: usize,
    pub total: usize,
    pub min_evidence: u32,
    pub confidence: Option<f64>,
    pub draft: Option<TrustDraft>,
}

/// Pending pre-authorisation for one request (C7). Actual settlement happens
/// in `record_cost`; a request that fails before any cost is released.
#[derive(Debug, Clone, Serialize)]
pub struct SpendReservation {
    pub id: String,
    pub key_id: Option<String>,
    pub estimated_usd: f64,
}

#[derive(Debug, Default)]
pub struct SpendState {
    pending: HashMap<String, SpendReservation>,
    seq: u64,
}

impl SpendState {
    pub fn open(&mut self, key_id: Option<&str>, estimated_usd: f64) -> String {
        self.seq += 1;
        let id = format!("spend_{}", self.seq);
        self.pending.insert(
            id.clone(),
            SpendReservation {
                id: id.clone(),
                key_id: key_id.map(str::to_string),
                estimated_usd,
            },
        );
        id
    }

    pub fn take(&mut self, id: &str) -> Option<SpendReservation> {
        self.pending.remove(id)
    }

    pub fn pending_for(&self, key_id: &str) -> f64 {
        self.pending
            .values()
            .filter(|r| r.key_id.as_deref() == Some(key_id))
            .map(|r| r.estimated_usd)
            .sum()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Debug, Default)]
pub struct RuntimeState {
    /// Last classified re-decision boundary per session (C1).
    boundaries: HashMap<String, String>,
    /// Request-scoped replay id, shared by the plain and rich records (C6).
    current_replay: HashMap<String, String>,
    /// Evidence seed captured from the request before forwarding (C6).
    evidence_seeds: HashMap<String, EvidenceSeed>,
    /// Latest execution evidence per session (C6).
    evidence: HashMap<String, StageEvidence>,
    /// Low-score protection state per session (C3).
    protection: HashMap<String, ProtectionState>,
    drafts: Vec<TrustDraft>,
    publications: Vec<TrustPublication>,
    seq: u64,
    pub spend: SpendState,
    /// Settlement failures are recorded (not swallowed) and exposed by
    /// `GET /v1/router/quota`. Bounded so a persist failure cannot grow memory.
    settlement_errors: Vec<String>,
}

impl RuntimeState {
    pub fn set_boundary(&mut self, session: &str, boundary: &str) {
        self.boundaries
            .insert(session.to_string(), boundary.to_string());
    }

    pub fn boundary(&self, session: &str) -> Option<String> {
        self.boundaries.get(session).cloned()
    }

    pub fn set_current_replay(&mut self, session: &str, id: &str) {
        self.current_replay
            .insert(session.to_string(), id.to_string());
    }

    pub fn current_replay(&self, session: &str) -> Option<String> {
        self.current_replay.get(session).cloned()
    }

    pub fn set_seed(&mut self, seed: EvidenceSeed) {
        self.evidence_seeds.insert(seed.session.clone(), seed);
    }

    pub fn seed(&self, session: &str) -> Option<EvidenceSeed> {
        self.evidence_seeds.get(session).cloned()
    }

    /// Fill in grading fields derived from the decision itself (C2 afm-d
    /// answers). Missing plan / rubric versions still leave evidence partial.
    pub fn record_settlement_error(&mut self, err: &RouterError) {
        if self.settlement_errors.len() >= 32 {
            self.settlement_errors.remove(0);
        }
        self.settlement_errors.push(err.to_string());
    }

    pub fn settlement_errors(&self) -> &[String] {
        &self.settlement_errors
    }

    pub fn amend_seed(&mut self, session: &str, score: Option<f32>, continuation: Option<f32>) {
        if let Some(s) = self.evidence_seeds.get_mut(session) {
            if s.score.is_none() {
                s.score = score;
            }
            if s.continuation.is_none() {
                s.continuation = continuation;
            }
        }
    }

    pub fn put_evidence(&mut self, ev: StageEvidence) {
        self.evidence.insert(ev.session.clone(), ev);
    }

    pub fn evidence_for(&self, session: &str) -> Option<&StageEvidence> {
        self.evidence.get(session)
    }

    pub fn all_evidence(&self) -> Vec<StageEvidence> {
        self.evidence.values().cloned().collect()
    }

    pub fn protection(&self, session: &str) -> Option<ProtectionState> {
        self.protection.get(session).cloned()
    }

    pub fn set_protection(&mut self, session: &str, state: ProtectionState) {
        self.protection.insert(session.to_string(), state);
    }

    pub fn drafts(&self) -> &[TrustDraft] {
        &self.drafts
    }

    pub fn publications(&self) -> &[TrustPublication] {
        &self.publications
    }

    pub fn published(&self) -> Option<&TrustPublication> {
        self.publications.last()
    }

    /// Build a suggestion from saved evidence only. `entrypoint` is the
    /// entrypoint currently being evaluated: records from another entrypoint
    /// are excluded (cross-entrypoint evidence is not comparable).
    pub fn suggest_trust(
        &mut self,
        cfg: &TrustCfg,
        evidence: &[StageEvidence],
        entrypoint: &str,
        now: f64,
    ) -> TrustOutcome {
        let total = evidence.len();
        if !cfg.enabled {
            return TrustOutcome {
                status: "disabled".into(),
                usable: 0,
                total,
                min_evidence: cfg.min_evidence,
                confidence: None,
                draft: None,
            };
        }
        let usable: Vec<&StageEvidence> = evidence
            .iter()
            .filter(|e| {
                e.entrypoint == entrypoint
                    && e.outcome == "ok"
                    && e.complete
                    && e.score.is_some()
                    && e.age_secs(now) <= 604_800
                    && cfg
                        .plan_version
                        .as_deref()
                        .is_none_or(|v| e.plan_version.as_deref() == Some(v))
                    && cfg
                        .rubric_version
                        .as_deref()
                        .is_none_or(|v| e.rubric_version.as_deref() == Some(v))
            })
            .collect();
        if usable.len() < cfg.min_evidence as usize {
            return TrustOutcome {
                status: "insufficient_evidence".into(),
                usable: usable.len(),
                total,
                min_evidence: cfg.min_evidence,
                confidence: None,
                draft: None,
            };
        }
        let mean = usable
            .iter()
            .filter_map(|e| e.score.map(f64::from))
            .sum::<f64>()
            / usable.len() as f64;
        if mean < f64::from(cfg.min_confidence) {
            return TrustOutcome {
                status: "low_confidence".into(),
                usable: usable.len(),
                total,
                min_evidence: cfg.min_evidence,
                confidence: Some(mean),
                draft: None,
            };
        }
        let (tier, action) = if mean >= f64::from(cfg.promote_at) {
            ("aggressive", "promote")
        } else if mean < f64::from(cfg.demote_below) {
            ("conservative", "demote")
        } else {
            ("balanced", "hold")
        };
        self.seq += 1;
        let draft = TrustDraft {
            id: format!("draft_{}", self.seq),
            created_at: now,
            entrypoint: entrypoint.to_string(),
            tier: tier.into(),
            action: action.into(),
            confidence: mean,
            sample_size: usable.len(),
            reasons: vec![
                "replay evidence only; no live routing changed".into(),
                format!("{} scored samples from {entrypoint}", usable.len()),
            ],
            published: false,
        };
        self.drafts.push(draft.clone());
        TrustOutcome {
            status: "ready".into(),
            usable: usable.len(),
            total,
            min_evidence: cfg.min_evidence,
            confidence: Some(mean),
            draft: Some(draft),
        }
    }

    /// Explicit publication. A draft id and an explicit confirmation are both
    /// required; publishing never touches `RouterDocument` / online routing.
    pub fn publish_trust(
        &mut self,
        draft_id: &str,
        confirm: bool,
        now: f64,
    ) -> Result<TrustPublication, RouterError> {
        if !confirm {
            return Err(RouterError::InvalidParam(
                "publication requires confirm: true".into(),
            ));
        }
        let draft = self
            .drafts
            .iter_mut()
            .find(|d| d.id == draft_id)
            .ok_or_else(|| {
                RouterError::InvalidParam(format!("trust draft {draft_id} not found"))
            })?;
        if draft.published {
            return Err(RouterError::InvalidParam(format!(
                "trust draft {draft_id} already published"
            )));
        }
        draft.published = true;
        self.seq += 1;
        let publication = TrustPublication {
            id: format!("pub_{}", self.seq),
            draft_id: draft.id.clone(),
            version: self.publications.len() as u64 + 1,
            tier: draft.tier.clone(),
            action: draft.action.clone(),
            confidence: draft.confidence,
            published_at: now,
        };
        self.publications.push(publication.clone());
        Ok(publication)
    }
}

/// Model / tier performance aggregation (C5).
///
/// Costs are actual ledger amounts; `notional_*` values are a heuristic
/// estimate (small=16, mid=4, large=1) and are labelled as such in the UI.
/// `has_evidence == false` must be rendered as "no data", never as zero.
pub fn model_performance(events: &[CostEvent], evidence: &[StageEvidence]) -> Value {
    #[derive(Default)]
    struct Agg {
        requests: u64,
        tokens: u64,
        cost_usd: f64,
        notional_usd: f64,
        scored: u64,
        score_sum: f64,
    }
    fn weight(tier: &str) -> f64 {
        match tier {
            "small" => 16.0,
            "mid" => 4.0,
            "large" => 1.0,
            _ => 4.0,
        }
    }
    let mut by_model: HashMap<String, Agg> = HashMap::new();
    let mut by_tier: HashMap<String, Agg> = HashMap::new();
    for e in events {
        let tier = e
            .competence_tier
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let tokens = e.prompt_tokens + e.completion_tokens;
        let a = by_model.entry(e.model.clone()).or_default();
        a.requests += 1;
        a.tokens += tokens;
        a.cost_usd += e.cost_usd;
        a.notional_usd += e.cost_usd * weight(&tier);
        let t = by_tier.entry(tier.clone()).or_default();
        t.requests += 1;
        t.tokens += tokens;
        t.cost_usd += e.cost_usd;
        t.notional_usd += e.cost_usd * weight(&tier);
    }
    for e in evidence {
        let Some(score) = e.score else { continue };
        if let Some(a) = by_model.get_mut(&e.model) {
            a.scored += 1;
            a.score_sum += f64::from(score);
        }
        if let Some(a) = by_tier.get_mut(&e.tier) {
            a.scored += 1;
            a.score_sum += f64::from(score);
        }
    }
    let total = events.len() as f64;
    let row = |name: &str, a: &Agg| {
        json!({
            "name": name,
            "requests": a.requests,
            "share": if total > 0.0 { a.requests as f64 / total } else { 0.0 },
            "tokens": a.tokens,
            "cost_usd": a.cost_usd,
            "notional_cost_usd": a.notional_usd,
            "stage_score": if a.scored > 0 { Some(a.score_sum / a.scored as f64) } else { None },
            "scored": a.scored,
        })
    };
    let mut models: Vec<Value> = by_model.iter().map(|(k, v)| row(k, v)).collect();
    models.sort_by(|a, b| {
        b["requests"]
            .as_u64()
            .cmp(&a["requests"].as_u64())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let mut tiers: Vec<Value> = by_tier.iter().map(|(k, v)| row(k, v)).collect();
    tiers.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let scored = evidence.iter().filter(|e| e.score.is_some()).count();
    json!({
        "by_model": models,
        "by_tier": tiers,
        "requests": events.len(),
        "evidence_count": evidence.len(),
        "scored_count": scored,
        "has_evidence": scored > 0,
        "estimate_note": "notional_cost_usd uses heuristic tier weights (small 16 / mid 4 / large 1); it is not real provider pricing.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, score: Option<f32>, entrypoint: &str, now: f64) -> StageEvidence {
        StageEvidence {
            replay_id: id.into(),
            session: format!("s-{id}"),
            entrypoint: entrypoint.into(),
            model: "local/small".into(),
            tier: "small".into(),
            task_category: "draft".into(),
            score,
            plan_version: Some("plan-1".into()),
            rubric_version: Some("rubric-1".into()),
            continuation: Some(0.9),
            complete: true,
            ts_unix: now,
            outcome: "ok".into(),
            ..StageEvidence::default()
        }
    }

    #[test]
    fn evidence_age_and_outcome_are_explicit() {
        let e = ev("r1", Some(0.8), "ep", 100.0);
        assert_eq!(e.age_secs(150.0), 50);
        assert_eq!(e.age_secs(50.0), 0);
        let p = e.to_protection_evidence(150.0);
        assert_eq!(p.key, "r1");
        assert!(p.complete);
        assert_eq!(p.score, Some(0.8));
        let mut bad = e;
        bad.outcome = "interrupted".into();
        assert!(!bad.to_protection_evidence(150.0).complete);
        assert_eq!(preview("abcdef", 3), "abc");
    }

    #[test]
    fn trust_requires_evidence_and_explicit_publication() {
        let mut st = RuntimeState::default();
        let cfg = TrustCfg {
            enabled: true,
            min_evidence: 2,
            min_confidence: 0.3,
            plan_version: Some("plan-1".into()),
            rubric_version: Some("rubric-1".into()),
            ..TrustCfg::default()
        };
        let now = 10_000.0;
        let empty = st.suggest_trust(&cfg, &[], "ep", now);
        assert_eq!(empty.status, "insufficient_evidence");
        assert!(empty.draft.is_none());
        // Cross-entrypoint and unscored / stale / wrong-plan records are excluded.
        let mut other = ev("r1", Some(0.9), "other", now);
        let mut unscored = ev("r2", None, "ep", now);
        let mut stale = ev("r3", Some(0.9), "ep", now - 604_801.0);
        let mut wrong_plan = ev("r4", Some(0.9), "ep", now);
        other.session = "s-other".into();
        unscored.session = "s-unscored".into();
        stale.session = "s-stale".into();
        wrong_plan.session = "s-plan".into();
        wrong_plan.plan_version = Some("plan-2".into());
        let bad = vec![other, unscored, stale, wrong_plan];
        assert_eq!(
            st.suggest_trust(&cfg, &bad, "ep", now).status,
            "insufficient_evidence"
        );
        let mut a = ev("r5", Some(0.9), "ep", now);
        let mut b = ev("r6", Some(0.95), "ep", now);
        a.session = "s-a".into();
        b.session = "s-b".into();
        let good = vec![a, b];
        let out = st.suggest_trust(&cfg, &good, "ep", now);
        assert_eq!(out.status, "ready");
        let draft = out.draft.unwrap();
        assert_eq!(draft.tier, "aggressive");
        assert_eq!(draft.action, "promote");
        assert_eq!(st.drafts().len(), 1);
        // No implicit publication and no config mutation.
        assert!(st.published().is_none());
        assert!(matches!(
            st.publish_trust(&draft.id, false, now),
            Err(RouterError::InvalidParam(_))
        ));
        assert!(matches!(
            st.publish_trust("missing", true, now),
            Err(RouterError::InvalidParam(_))
        ));
        let pub_ = st.publish_trust(&draft.id, true, now).unwrap();
        assert_eq!(pub_.version, 1);
        assert_eq!(st.publications().len(), 1);
        assert!(st.published().is_some());
        assert!(matches!(
            st.publish_trust(&draft.id, true, now),
            Err(RouterError::InvalidParam(_))
        ));
        let off = TrustCfg::default();
        assert_eq!(st.suggest_trust(&off, &good, "ep", now).status, "disabled");
    }

    #[test]
    fn trust_gate_rejects_low_confidence_and_hold_is_balanced() {
        let mut st = RuntimeState::default();
        let cfg = TrustCfg {
            enabled: true,
            min_evidence: 1,
            min_confidence: 0.5,
            ..TrustCfg::default()
        };
        let now = 100.0;
        let low = vec![ev("r1", Some(0.1), "ep", now)];
        let out = st.suggest_trust(&cfg, &low, "ep", now);
        assert_eq!(out.status, "low_confidence");
        assert!(out.draft.is_none());
        let mid = vec![ev("r2", Some(0.6), "ep", now)];
        let out = st.suggest_trust(&cfg, &mid, "ep", now);
        assert_eq!(out.status, "ready");
        assert_eq!(out.draft.unwrap().tier, "balanced");
        let demote = vec![ev("r3", Some(0.51), "ep", now)];
        let cfg2 = TrustCfg {
            min_confidence: 0.0,
            demote_below: 0.6,
            ..cfg
        };
        let out = st.suggest_trust(&cfg2, &demote, "ep", now);
        assert_eq!(out.draft.unwrap().action, "demote");
    }

    #[test]
    fn runtime_state_tracks_boundaries_evidence_and_protection() {
        let mut st = RuntimeState::default();
        st.set_boundary("s1", "same-turn");
        assert_eq!(st.boundary("s1").as_deref(), Some("same-turn"));
        assert_eq!(st.boundary("missing"), None);
        let e = ev("r1", Some(0.2), "ep", 10.0);
        st.put_evidence(e.clone());
        assert_eq!(st.evidence_for("s-r1").unwrap().score, Some(0.2));
        assert_eq!(st.all_evidence().len(), 1);
        st.set_protection(
            "s-r1",
            ProtectionState {
                active: true,
                ..ProtectionState::default()
            },
        );
        assert!(st.protection("s-r1").unwrap().active);
    }

    #[test]
    fn spend_reservations_open_take_and_sum() {
        let mut s = SpendState::default();
        let a = s.open(Some("key_1"), 0.5);
        let b = s.open(Some("key_1"), 0.25);
        s.open(Some("key_2"), 1.0);
        assert_eq!(s.len(), 3);
        assert!((s.pending_for("key_1") - 0.75).abs() < 1e-9);
        let taken = s.take(&a).unwrap();
        assert_eq!(taken.key_id.as_deref(), Some("key_1"));
        assert!(s.take(&a).is_none());
        assert!(s.take(&b).is_some());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn model_performance_reports_no_data_and_estimates() {
        let empty = model_performance(&[], &[]);
        assert_eq!(empty["has_evidence"], false);
        assert_eq!(empty["by_model"].as_array().unwrap().len(), 0);
        let now = now_unix();
        let e = StageEvidence {
            model: "local/small".into(),
            tier: "small".into(),
            score: Some(0.4),
            ..ev("r1", Some(0.4), "ep", now)
        };
        let event = CostEvent {
            model: "local/small".into(),
            competence_tier: Some("small".into()),
            cost_usd: 0.25,
            prompt_tokens: 10,
            completion_tokens: 5,
            ..CostEvent::default()
        };
        let out = model_performance(&[event], &[e]);
        let rows = out["by_model"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["requests"], 1);
        assert_eq!(rows[0]["scored"], 1);
        assert_eq!(out["by_tier"].as_array().unwrap().len(), 1);
        assert_eq!(out["has_evidence"], true);
        assert!(out["estimate_note"].as_str().unwrap().contains("heuristic"));
    }

    #[test]
    fn amend_seed_only_fills_missing_fields() {
        let mut st = RuntimeState::default();
        st.set_seed(EvidenceSeed {
            session: "s".into(),
            ..Default::default()
        });
        st.amend_seed("absent", Some(0.9), Some(0.9));
        st.amend_seed("s", Some(0.4), Some(0.2));
        assert_eq!(st.seed("s").unwrap().score, Some(0.4));
        assert_eq!(st.seed("s").unwrap().continuation, Some(0.2));
        st.amend_seed("s", Some(0.9), Some(0.9));
        assert_eq!(st.seed("s").unwrap().score, Some(0.4));
        assert_eq!(st.seed("s").unwrap().continuation, Some(0.2));
    }

    #[test]
    fn settlement_errors_are_recorded_and_bounded() {
        let mut st = RuntimeState::default();
        assert!(st.settlement_errors().is_empty());
        for i in 0..40 {
            st.record_settlement_error(&RouterError::Upstream(format!("failure-{i}")));
        }
        assert_eq!(st.settlement_errors().len(), 32);
        assert!(st.settlement_errors()[0].contains("failure-8"));
        assert!(st.settlement_errors()[31].contains("failure-39"));
    }

    #[test]
    fn model_performance_breaks_equal_request_counts_by_name() {
        let a = CostEvent {
            model: "b/model".into(),
            ..Default::default()
        };
        let b = CostEvent {
            model: "a/model".into(),
            ..Default::default()
        };
        let out = model_performance(&[a, b], &[]);
        let rows = out["by_model"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "a/model");
        assert_eq!(rows[1]["name"], "b/model");
    }
}
