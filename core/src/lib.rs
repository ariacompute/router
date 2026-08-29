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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
