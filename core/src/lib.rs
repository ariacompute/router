//! Shared types and `RouterError`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RouterError {
    #[error("io: {0}")]
    Io(String),
    #[error("config: {0}")]
    Config(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid param: {0}")]
    InvalidParam(String),
    #[error("fail closed: {0}")]
    FailClosed(String),
    #[error("upstream: {0}")]
    Upstream(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("extension: {0}")]
    Extension(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
}

impl From<std::io::Error> for RouterError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for RouterError {
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidParam(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouterKind {
    Semantic,
    Agent,
    #[serde(rename = "afm-d")]
    AfmD,
}

impl RouterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Agent => "agent",
            Self::AfmD => "afm-d",
        }
    }
}

impl std::str::FromStr for RouterKind {
    type Err = RouterError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "semantic" => Ok(Self::Semantic),
            "agent" => Ok(Self::Agent),
            "afm-d" => Ok(Self::AfmD),
            other => Err(RouterError::Config(format!(
                "router must be semantic|agent|afm-d, got {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RouteDecision {
    pub model: String,
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub layer: String,
    #[serde(default)]
    pub decision: String,
    #[serde(default)]
    pub bypass: bool,
    /// Router-side decision latency in milliseconds (excludes upstream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_latency_ms: Option<f64>,
    /// When true, non-stream responses may be stored in response-cache.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cache_response: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_drop: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_ttl_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_keep_current_model: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_prefer_prefix: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_cache_hit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

fn default_confidence() -> f32 {
    1.0
}

impl RouteDecision {
    pub fn bypass(model: &str) -> Self {
        Self {
            model: model.to_string(),
            algorithm: Some("static".into()),
            reason: "passthrough".into(),
            confidence: 1.0,
            layer: "bypass".into(),
            decision: "passthrough".into(),
            bypass: true,
            routing_latency_ms: None,
            cache_response: false,
            retention_drop: None,
            retention_ttl_turns: None,
            retention_keep_current_model: None,
            retention_prefer_prefix: None,
            semantic_cache_hit: None,
            replay_id: None,
            session: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: serde_json::Value,
}

impl ChatMessage {
    pub fn text(&self) -> String {
        match &self.content {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ChatRequest {
    pub fn prompt_text(&self) -> String {
        self.messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Latest user turn only — keyword / language / structure eligibility for chat.
    pub fn last_user_text(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.text())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCard {
    pub name: String,
    #[serde(default = "default_locality")]
    pub locality: String,
    #[serde(default = "default_modality")]
    pub modality: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub provider_model_id: String,
    /// Explicit pool tier (`small` / `mid` / `large`); preferred over name heuristics.
    #[serde(default)]
    pub tier: Option<String>,
}

fn default_locality() -> String {
    "local".into()
}
fn default_modality() -> String {
    "text".into()
}

#[derive(Debug, Clone, Default)]
pub struct ConstraintCtx {
    pub require_locality: Option<String>,
    pub require_modality: Option<String>,
    pub authz_ok: bool,
}

impl ConstraintCtx {
    pub fn open() -> Self {
        Self {
            require_locality: None,
            require_modality: Some("text".into()),
            authz_ok: true,
        }
    }
}

pub fn prompt_from_messages(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.text()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Per-session context fingerprint used to detect compaction / rebuild.
#[derive(Debug, Clone, Copy)]
struct SessionCtx {
    msg_len: usize,
    prefix_fp: u64,
}

/// Per-(session, turn) decision cache for context-prefix reuse.
///
/// Within the same agent turn (same session + same count of user messages) the
/// frozen `RouteDecision` is reused instead of re-deciding; a new user turn (or
/// unrecognizable context) forces a fresh decision. Keys are evicted FIFO once
/// `max` is exceeded. Compaction (message count drop or system/tools prefix
/// change) clears all entries for that session.
#[derive(Debug)]
pub struct DecisionCache {
    inner: Mutex<HashMap<(String, u64), RouteDecision>>,
    session_ctx: Mutex<HashMap<String, SessionCtx>>,
    max: usize,
}

impl Default for DecisionCache {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl DecisionCache {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            session_ctx: Mutex::new(HashMap::new()),
            max: max.max(1),
        }
    }

    pub fn get(&self, session: &str, turn: u64) -> Option<RouteDecision> {
        self.inner
            .lock()
            .unwrap()
            .get(&(session.to_string(), turn))
            .cloned()
    }

    pub fn put(&self, session: String, turn: u64, d: RouteDecision) {
        let mut g = self.inner.lock().unwrap();
        let key = (session, turn);
        g.insert(key.clone(), d);
        if g.len() > self.max {
            // Evict an older entry (never the one just inserted) to bound memory.
            let victim = g.keys().find(|k| **k != key).cloned();
            if let Some(v) = victim {
                g.remove(&v);
            }
        }
    }

    /// Observe the current message list / tools for `session`.
    ///
    /// Returns `true` when context was compacted (message count dropped, or
    /// system-prompt / tools fingerprint changed). On compaction, all cached
    /// decisions for the session are cleared.
    pub fn observe_and_check_compact(
        &self,
        session: &str,
        messages: &[ChatMessage],
        tools: Option<&serde_json::Value>,
    ) -> bool {
        if session.is_empty() {
            return false;
        }
        let msg_len = messages.len();
        let prefix_fp = context_prefix_fp(messages, tools);
        let compacted = {
            let ctx = self.session_ctx.lock().unwrap();
            match ctx.get(session) {
                Some(prev) => msg_len < prev.msg_len || prefix_fp != prev.prefix_fp,
                None => false,
            }
        };
        if compacted {
            // Drop frozen decisions; fingerprint is rewritten as the new baseline below.
            self.inner
                .lock()
                .unwrap()
                .retain(|(s, _), _| s.as_str() != session);
        }
        self.session_ctx
            .lock()
            .unwrap()
            .insert(session.to_string(), SessionCtx { msg_len, prefix_fp });
        compacted
    }

    /// Drop every cached decision (and context fingerprint) for one session.
    pub fn clear_session(&self, session: &str) {
        self.inner
            .lock()
            .unwrap()
            .retain(|(s, _), _| s.as_str() != session);
        self.session_ctx.lock().unwrap().remove(session);
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
        self.session_ctx.lock().unwrap().clear();
    }
}

/// Hash of system messages + optional `tools` payload (agent tool definitions).
/// Used to detect prompt rebuild / compaction that should bust prefix reuse.
pub fn context_prefix_fp(messages: &[ChatMessage], tools: Option<&serde_json::Value>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for m in messages.iter().filter(|m| m.role == "system") {
        m.role.hash(&mut h);
        m.text().hash(&mut h);
    }
    if let Some(t) = tools {
        t.to_string().hash(&mut h);
    }
    h.finish()
}

/// Number of user messages in a request; used as the "turn" key for prefix reuse.
pub fn turn_of(messages: &[ChatMessage]) -> u64 {
    messages.iter().filter(|m| m.role == "user").count() as u64
}

/// Per-session rolling competence tracker.
///
/// Stage competence scores (0..1) are folded into a rolling estimate per
/// session via exponential moving average. `score` returns `None` until at least
/// one sample has been recorded, which callers use to *forbid* downgrade when
/// competence is unknown.
#[derive(Debug, Default)]
pub struct CompetenceTracker {
    inner: Mutex<HashMap<String, f32>>,
}

impl CompetenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, session: &str, stage: f32) {
        let mut g = self.inner.lock().unwrap();
        let next = match g.get(session) {
            Some(&prev) => 0.7 * prev + 0.3 * stage,
            None => stage,
        };
        g.insert(session.to_string(), next.clamp(0.0, 1.0));
    }

    pub fn score(&self, session: &str) -> Option<f32> {
        self.inner.lock().unwrap().get(session).copied()
    }
}

/// Best-effort pool tier (`small`/`mid`/`large`) from a logical model name.
/// Prefers an explicit `tier` on the card where available; this helper only
/// inspects the name and is used to populate evidence (`selected_group`).
pub fn tier_of_model(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with("-large") || lower.contains("/ariamodel-large") || lower.ends_with("/large")
    {
        "large"
    } else if lower.ends_with("-mid")
        || lower.contains("/ariamodel-mid")
        || lower.ends_with("/mid")
        || lower.contains("-medium")
    {
        "mid"
    } else if lower.ends_with("-small")
        || lower.contains("/ariamodel-small")
        || lower.ends_with("/small")
    {
        "small"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chat_request_text_helpers() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: json!("sys"),
                },
                ChatMessage {
                    role: "user".into(),
                    content: json!("first"),
                },
                ChatMessage {
                    role: "assistant".into(),
                    content: json!("ok"),
                },
                ChatMessage {
                    role: "user".into(),
                    content: json!({"text": "second"}),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        assert!(req.last_user_text().contains("second"));
        assert!(req.prompt_text().contains("first"));
        let joined = prompt_from_messages(&req.messages);
        assert!(joined.contains("system: sys"));
        assert!(joined.contains("assistant: ok"));
    }

    #[test]
    fn router_kind_and_bypass() {
        assert_eq!(
            "semantic".parse::<RouterKind>().unwrap(),
            RouterKind::Semantic
        );
        assert_eq!("agent".parse::<RouterKind>().unwrap(), RouterKind::Agent);
        assert_eq!("afm-d".parse::<RouterKind>().unwrap(), RouterKind::AfmD);
        assert_eq!(RouterKind::Semantic.as_str(), "semantic");
        assert_eq!(RouterKind::AfmD.as_str(), "afm-d");
        let afm_json = serde_json::to_string(&RouterKind::AfmD).unwrap();
        assert_eq!(afm_json, "\"afm-d\"");
        assert_eq!(
            serde_json::from_str::<RouterKind>("\"afm-d\"").unwrap(),
            RouterKind::AfmD
        );
        assert!(matches!(
            "other".parse::<RouterKind>(),
            Err(RouterError::Config(_))
        ));
        let d = RouteDecision::bypass("local/m");
        assert!(d.bypass);
        assert_eq!(d.layer, "bypass");
        assert_eq!(d.model, "local/m");
        assert_eq!(
            ConstraintCtx::open().require_modality.as_deref(),
            Some("text")
        );
    }

    #[test]
    fn error_from_io_and_json() {
        let io = RouterError::from(std::io::Error::other("x"));
        assert!(matches!(io, RouterError::Io(_)));
        let bad: Result<i32, _> = serde_json::from_str("not-json");
        let e = RouterError::from(bad.unwrap_err());
        assert!(matches!(e, RouterError::InvalidParam(_)));
    }

    #[test]
    fn decision_cache_hit_miss_and_evict() {
        let cache = DecisionCache::new(2);
        let d = RouteDecision::bypass("local/general");
        assert!(cache.get("s", 1).is_none());
        cache.put("s".into(), 1, d.clone());
        let got = cache.get("s", 1).expect("should hit");
        assert_eq!(got.model, "local/general");
        // Different turn misses.
        assert!(cache.get("s", 2).is_none());
        // Eviction keeps behaviour bounded.
        cache.put("s".into(), 2, RouteDecision::bypass("local/a"));
        cache.put("s".into(), 3, RouteDecision::bypass("local/b"));
        cache.put("s".into(), 4, RouteDecision::bypass("local/c"));
        assert!(cache.get("s", 4).is_some());
        cache.clear();
        assert!(cache.get("s", 1).is_none());
    }

    #[test]
    fn decision_cache_compaction_clears_session() {
        let cache = DecisionCache::new(8);
        let long = vec![
            ChatMessage {
                role: "system".into(),
                content: json!("sys-a"),
            },
            ChatMessage {
                role: "user".into(),
                content: json!("u1"),
            },
            ChatMessage {
                role: "assistant".into(),
                content: json!("a1"),
            },
            ChatMessage {
                role: "user".into(),
                content: json!("u2"),
            },
        ];
        assert!(!cache.observe_and_check_compact("s1", &long, None));
        cache.put("s1".into(), 2, RouteDecision::bypass("m1"));
        assert!(cache.get("s1", 2).is_some());
        // Tool append grows length with same system → not compacted.
        let mut grown = long.clone();
        grown.push(ChatMessage {
            role: "tool".into(),
            content: json!("ok"),
        });
        assert!(!cache.observe_and_check_compact("s1", &grown, None));
        assert!(cache.get("s1", 2).is_some());
        // Shorter history → compacted, decisions cleared.
        let short = vec![
            ChatMessage {
                role: "system".into(),
                content: json!("sys-a"),
            },
            ChatMessage {
                role: "user".into(),
                content: json!("summary"),
            },
        ];
        assert!(cache.observe_and_check_compact("s1", &short, None));
        assert!(cache.get("s1", 2).is_none());
        // System prefix change → compacted.
        cache.put("s1".into(), 1, RouteDecision::bypass("m2"));
        let changed_sys = vec![
            ChatMessage {
                role: "system".into(),
                content: json!("sys-b"),
            },
            ChatMessage {
                role: "user".into(),
                content: json!("summary"),
            },
        ];
        assert!(cache.observe_and_check_compact("s1", &changed_sys, None));
        assert!(cache.get("s1", 1).is_none());
    }

    #[test]
    fn decision_cache_tools_fingerprint_and_clear_session() {
        let cache = DecisionCache::new(8);
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: json!("hi"),
        }];
        let tools_a = json!([{"name": "search"}]);
        let tools_b = json!([{"name": "browse"}]);
        assert!(!cache.observe_and_check_compact("s", &msgs, Some(&tools_a)));
        cache.put("s".into(), 1, RouteDecision::bypass("m1"));
        assert!(cache.get("s", 1).is_some());
        // Same messages, different tools → compacted.
        assert!(cache.observe_and_check_compact("s", &msgs, Some(&tools_b)));
        assert!(cache.get("s", 1).is_none());

        cache.put("s".into(), 1, RouteDecision::bypass("m2"));
        cache.put("other".into(), 1, RouteDecision::bypass("m3"));
        cache.clear_session("s");
        assert!(cache.get("s", 1).is_none());
        assert!(cache.get("other", 1).is_some());

        // Empty session never compacts.
        assert!(!cache.observe_and_check_compact("", &msgs, None));
    }

    #[test]
    fn context_prefix_fp_stable_and_tools_sensitive() {
        let msgs = vec![
            ChatMessage {
                role: "system".into(),
                content: json!("sys"),
            },
            ChatMessage {
                role: "user".into(),
                content: json!("u"),
            },
        ];
        let a = context_prefix_fp(&msgs, None);
        let b = context_prefix_fp(&msgs, None);
        assert_eq!(a, b);
        let with_tools = context_prefix_fp(&msgs, Some(&json!({"fn": 1})));
        assert_ne!(a, with_tools);
        // Non-system messages alone do not affect the fingerprint.
        let user_only = vec![ChatMessage {
            role: "user".into(),
            content: json!("other"),
        }];
        assert_eq!(
            context_prefix_fp(&user_only, None),
            context_prefix_fp(&[], None)
        );
    }

    #[test]
    fn turn_of_counts_user_messages() {
        use crate::ChatMessage;
        let msgs = vec![
            ChatMessage {
                role: "system".into(),
                content: serde_json::json!("s"),
            },
            ChatMessage {
                role: "user".into(),
                content: serde_json::json!("u1"),
            },
            ChatMessage {
                role: "assistant".into(),
                content: serde_json::json!("a"),
            },
            ChatMessage {
                role: "user".into(),
                content: serde_json::json!("u2"),
            },
        ];
        assert_eq!(turn_of(&msgs), 2);
    }

    #[test]
    fn competence_tracker_unknown_then_ema() {
        let t = CompetenceTracker::new();
        // Unknown session must forbid downgrade (callers treat None as "do not downgrade").
        assert!(t.score("s1").is_none());
        t.record("s1", 1.0);
        assert_eq!(t.score("s1"), Some(1.0));
        // EMA: 0.7*1.0 + 0.3*0.0 = 0.7, then clamps to [0,1].
        t.record("s1", 0.0);
        assert!((t.score("s1").unwrap() - 0.7).abs() < 1e-6);
        // Sessions are tracked independently.
        assert!(t.score("s2").is_none());
    }

    #[test]
    fn tier_of_model_classifies_names() {
        assert_eq!(tier_of_model("ariacompute/ariamodel-large"), "large");
        assert_eq!(tier_of_model("x/y-large"), "large");
        assert_eq!(tier_of_model("m-mid"), "mid");
        assert_eq!(tier_of_model("m-medium"), "mid");
        assert_eq!(tier_of_model("m-small"), "small");
        assert_eq!(tier_of_model("local/general"), "unknown");
    }
}
