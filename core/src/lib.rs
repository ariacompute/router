//! Shared types and `RouterError`.

use serde::{Deserialize, Serialize};
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
}

impl RouterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Agent => "agent",
        }
    }
}

impl std::str::FromStr for RouterKind {
    type Err = RouterError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "semantic" => Ok(Self::Semantic),
            "agent" => Ok(Self::Agent),
            other => Err(RouterError::Config(format!("router must be semantic|agent, got {other}"))),
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
        assert_eq!(req.last_user_text().contains("second"), true);
        assert!(req.prompt_text().contains("first"));
        let joined = prompt_from_messages(&req.messages);
        assert!(joined.contains("system: sys"));
        assert!(joined.contains("assistant: ok"));
    }

    #[test]
    fn router_kind_and_bypass() {
        assert_eq!("semantic".parse::<RouterKind>().unwrap(), RouterKind::Semantic);
        assert_eq!("agent".parse::<RouterKind>().unwrap(), RouterKind::Agent);
        assert_eq!(RouterKind::Semantic.as_str(), "semantic");
        assert!(matches!(
            "other".parse::<RouterKind>(),
            Err(RouterError::Config(_))
        ));
        let d = RouteDecision::bypass("local/m");
        assert!(d.bypass);
        assert_eq!(d.layer, "bypass");
        assert_eq!(d.model, "local/m");
        assert_eq!(ConstraintCtx::open().require_modality.as_deref(), Some("text"));
    }

    #[test]
    fn error_from_io_and_json() {
        let io = RouterError::from(std::io::Error::other("x"));
        assert!(matches!(io, RouterError::Io(_)));
        let bad: Result<i32, _> = serde_json::from_str("not-json");
        let e = RouterError::from(bad.unwrap_err());
        assert!(matches!(e, RouterError::InvalidParam(_)));
    }
}
