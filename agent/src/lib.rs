//! Agent router: extension trait + builtin tool-call loop.

use aria_router_config::{AgentRecipe, ExtensionCfg};
use aria_router_core::{ChatRequest, ModelCard, RouteDecision, RouterError};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct RouteTask {
    pub prompt: String,
    pub eligible: Vec<ModelCard>,
    pub system: String,
    pub timeout_ms: u64,
    pub max_turns: u32,
}

#[async_trait]
pub trait AgentExtension: Send + Sync {
    fn name(&self) -> &str;
    async fn route(&self, task: RouteTask) -> Result<RouteDecision, RouterError>;
    async fn shutdown(&self) -> Result<(), RouterError> {
        Ok(())
    }
}

/// Injected decision for tests (no LLM).
pub struct FakeExtension {
    pub name: String,
    pub decision: Result<RouteDecision, RouterError>,
}

#[async_trait]
impl AgentExtension for FakeExtension {
    fn name(&self) -> &str {
        &self.name
    }
    async fn route(&self, task: RouteTask) -> Result<RouteDecision, RouterError> {
        match &self.decision {
            Ok(d) => {
                if !task.eligible.iter().any(|m| m.name == d.model) {
                    return Err(RouterError::FailClosed(format!(
                        "agent chose {} not in eligible pool",
                        d.model
                    )));
                }
                Ok(d.clone())
            }
            Err(e) => Err(e.clone()),
        }
    }
}

/// Builtin: OpenAI-compatible tool loop, or single-shot JSON if `ARIA_ROUTER_BUILTIN_JSON` style.
pub struct BuiltinExtension {
    pub endpoint: Option<String>,
    pub model: String,
    pub canned: Option<RouteDecision>,
}

#[async_trait]
impl AgentExtension for BuiltinExtension {
    fn name(&self) -> &str {
        "builtin"
    }
    async fn route(&self, task: RouteTask) -> Result<RouteDecision, RouterError> {
        if let Some(d) = &self.canned {
            if !task.eligible.iter().any(|m| m.name == d.model) {
                return Err(RouterError::FailClosed(format!(
                    "agent chose {} not in eligible pool",
                    d.model
                )));
            }
            return Ok(d.clone());
        }
        if task.eligible.is_empty() {
            return Err(RouterError::FailClosed("no eligible models".into()));
        }
        let Some(endpoint) = &self.endpoint else {
            // No LLM: pick first eligible (still a real policy, documented for CI without keys).
            return Ok(RouteDecision {
                model: task.eligible[0].name.clone(),
                algorithm: Some("static".into()),
                reason: "builtin:first-eligible".into(),
                confidence: 0.5,
                layer: "agent".into(),
                decision: "builtin".into(),
                bypass: false,
            });
        };
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": task.system },
                { "role": "user", "content": format!(
                    "Eligible models: {}\nRequest:\n{}\nReturn JSON {{\"model\":\"...\",\"reason\":\"...\",\"confidence\":0.8}}",
                    task.eligible.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "),
                    task.prompt
                )}
            ],
            "temperature": 0
        });
        let resp = tokio::time::timeout(
            std::time::Duration::from_millis(task.timeout_ms),
            reqwest::Client::new()
                .post(format!("{}/v1/chat/completions", endpoint.trim_end_matches('/')))
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| RouterError::Timeout("builtin agent".into()))?
        .map_err(|e| RouterError::Extension(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|e| RouterError::Extension(e.to_string()))?;
        let content = v
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        parse_decision_json(content, &task.eligible)
    }
}

pub fn parse_decision_json(raw: &str, eligible: &[ModelCard]) -> Result<RouteDecision, RouterError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let v: Value = serde_json::from_str(trimmed).map_err(|_| {
        RouterError::FailClosed(format!("agent output is not JSON: {trimmed}"))
    })?;
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .ok_or_else(|| RouterError::FailClosed("agent JSON missing model".into()))?
        .to_string();
    if !eligible.iter().any(|m| m.name == model) {
        return Err(RouterError::FailClosed(format!(
            "agent chose {model} not in eligible pool"
        )));
    }
    Ok(RouteDecision {
        model,
        algorithm: v
            .get("algorithm")
            .and_then(|a| a.as_str())
            .map(|s| s.to_string()),
        reason: v
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("agent")
            .to_string(),
        confidence: v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.8) as f32,
        layer: "agent".into(),
        decision: "agent".into(),
        bypass: false,
    })
}

pub fn task_from(
    req: &ChatRequest,
    eligible: Vec<ModelCard>,
    agent: &AgentRecipe,
    ext: &ExtensionCfg,
) -> RouteTask {
    RouteTask {
        prompt: req.prompt_text(),
        eligible,
        system: agent
            .prompt
            .clone()
            .unwrap_or_else(|| "Choose one eligible model. Reply with JSON only.".into()),
        timeout_ms: agent.timeout_ms.or(ext.timeout_ms).unwrap_or(8000),
        max_turns: agent.max_turns.unwrap_or(4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_non_json() {
        let cards = vec![ModelCard {
            name: "local/general".into(),
            locality: "local".into(),
            modality: "text".into(),
            capabilities: vec![],
            provider_model_id: "x".into(),
        }];
        assert!(parse_decision_json("not json", &cards).is_err());
    }

    #[test]
    fn reject_unknown_model() {
        let cards = vec![ModelCard {
            name: "local/general".into(),
            locality: "local".into(),
            modality: "text".into(),
            capabilities: vec![],
            provider_model_id: "x".into(),
        }];
        assert!(parse_decision_json(r#"{"model":"cloud/x"}"#, &cards).is_err());
    }

    #[test]
    fn accept_json() {
        let cards = vec![ModelCard {
            name: "local/general".into(),
            locality: "local".into(),
            modality: "text".into(),
            capabilities: vec![],
            provider_model_id: "x".into(),
        }];
        let d = parse_decision_json(r#"{"model":"local/general","reason":"ok","confidence":0.9}"#, &cards).unwrap();
        assert_eq!(d.model, "local/general");
    }
}
