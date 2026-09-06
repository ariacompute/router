//! Lightweight in-process builtin agent: fixed tools + limited turns.

use aria_router_config::AgentRecipe;
use aria_router_core::{ChatRequest, ModelCard, RouteDecision, RouterError};
use serde_json::{json, Value};
use std::collections::HashMap;

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const DEFAULT_MAX_TURNS: u32 = 3;
const MAX_TURNS_CLAMP: u32 = 8;

const DEFAULT_SYSTEM: &str = "You are the aria-router builtin agent. \
Use tools if needed, then call submit_route with one eligible model. JSON tools only.";

#[derive(Debug, Clone)]
pub struct RouteTask {
    pub prompt: String,
    pub eligible: Vec<ModelCard>,
    pub system: String,
    pub timeout_ms: u64,
    pub max_turns: u32,
}

/// Snapshot of pool + request for in-process tools (no I/O).
#[derive(Debug, Clone, Default)]
pub struct ToolRuntime {
    pub latency_ms: HashMap<String, f32>,
    pub failures: HashMap<String, u32>,
    pub request_view: Value,
}

/// In-process OpenAI-compatible tool loop.
pub struct BuiltinAgent {
    pub endpoint: Option<String>,
    pub model: String,
    pub canned: Option<RouteDecision>,
}

impl BuiltinAgent {
    pub async fn route(
        &self,
        task: RouteTask,
        tools: &ToolRuntime,
    ) -> Result<RouteDecision, RouterError> {
        if let Some(d) = &self.canned {
            return validate_decision(d.clone(), &task.eligible);
        }
        if task.eligible.is_empty() {
            return Err(RouterError::FailClosed("no eligible models".into()));
        }
        let Some(endpoint) = &self.endpoint else {
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

        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(task.timeout_ms.max(1));
        let client = reqwest::Client::new();
        let url = format!(
            "{}/v1/chat/completions",
            endpoint.trim_end_matches('/')
        );
        let mut messages = vec![
            json!({"role": "system", "content": task.system}),
            json!({
                "role": "user",
                "content": format!(
                    "Route this request.\nEligible (also via list_eligible_models): {}\nRequest:\n{}",
                    task.eligible.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "),
                    task.prompt
                )
            }),
        ];
        let tool_defs = tool_definitions();

        for turn in 0..task.max_turns {
            if std::time::Instant::now() >= deadline {
                return Err(RouterError::Timeout("builtin agent".into()));
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let body = json!({
                "model": self.model,
                "messages": messages,
                "tools": tool_defs,
                "tool_choice": "auto",
                "temperature": 0
            });
            let resp = tokio::time::timeout(
                remaining,
                client.post(&url).json(&body).send(),
            )
            .await
            .map_err(|_| RouterError::Timeout("builtin agent".into()))?
            .map_err(|e| RouterError::Extension(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(RouterError::Extension(format!(
                    "builtin LLM {status}: {text}"
                )));
            }
            let v: Value = resp
                .json()
                .await
                .map_err(|e| RouterError::Extension(e.to_string()))?;
            let msg = v
                .pointer("/choices/0/message")
                .cloned()
                .ok_or_else(|| RouterError::FailClosed("builtin LLM missing message".into()))?;
            messages.push(msg.clone());

            if let Some(calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                if calls.is_empty() {
                    // fall through to content parse
                } else {
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("call")
                            .to_string();
                        let name = call
                            .pointer("/function/name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        let args_raw = call
                            .pointer("/function/arguments")
                            .and_then(|x| x.as_str())
                            .unwrap_or("{}");
                        let args: Value = serde_json::from_str(args_raw).unwrap_or(json!({}));
                        if name == "submit_route" {
                            return decision_from_submit(&args, &task.eligible);
                        }
                        let result = run_tool(name, &args, &task.eligible, tools)?;
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": result.to_string()
                        }));
                    }
                    if turn + 1 >= task.max_turns {
                        return Err(RouterError::FailClosed(format!(
                            "builtin agent exceeded max_turns ({})",
                            task.max_turns
                        )));
                    }
                    continue;
                }
            }

            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
            if !content.trim().is_empty() {
                return parse_decision_json(content, &task.eligible);
            }
            return Err(RouterError::FailClosed(format!(
                "builtin agent turn {} produced no tool_calls or content",
                turn + 1
            )));
        }
        Err(RouterError::FailClosed(format!(
            "builtin agent exceeded max_turns ({})",
            task.max_turns
        )))
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "list_eligible_models",
                "description": "List models that passed hard constraints",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "get_backend_health",
                "description": "Failure counts for eligible backends",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "model": { "type": "string", "description": "optional model name filter" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "get_recent_latency",
                "description": "Recent latency_ms samples for eligible models",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "model": { "type": "string" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "get_request_view",
                "description": "Redacted view of the inbound request",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "submit_route",
                "description": "Finalize routing with one eligible model",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "model": { "type": "string" },
                        "reason": { "type": "string" },
                        "confidence": { "type": "number" },
                        "algorithm": { "type": "string" }
                    },
                    "required": ["model"]
                }
            }
        }
    ])
}

fn run_tool(
    name: &str,
    args: &Value,
    eligible: &[ModelCard],
    tools: &ToolRuntime,
) -> Result<Value, RouterError> {
    match name {
        "list_eligible_models" => Ok(json!(eligible
            .iter()
            .map(|m| json!({
                "name": m.name,
                "locality": m.locality,
                "modality": m.modality,
                "capabilities": m.capabilities,
            }))
            .collect::<Vec<_>>())),
        "get_backend_health" => {
            let filter = args.get("model").and_then(|m| m.as_str());
            let mut out = serde_json::Map::new();
            for m in eligible {
                if filter.is_some_and(|f| f != m.name) {
                    continue;
                }
                let fails = tools.failures.get(&m.name).copied().unwrap_or(0);
                out.insert(
                    m.name.clone(),
                    json!({ "failures": fails, "healthy": fails == 0 }),
                );
            }
            Ok(Value::Object(out))
        }
        "get_recent_latency" => {
            let filter = args.get("model").and_then(|m| m.as_str());
            let mut out = serde_json::Map::new();
            for m in eligible {
                if filter.is_some_and(|f| f != m.name) {
                    continue;
                }
                if let Some(ms) = tools.latency_ms.get(&m.name) {
                    out.insert(m.name.clone(), json!(ms));
                }
            }
            Ok(Value::Object(out))
        }
        "get_request_view" => Ok(tools.request_view.clone()),
        "submit_route" => Err(RouterError::InvalidParam(
            "submit_route must be handled as terminal".into(),
        )),
        other => Err(RouterError::Unsupported(format!("unknown tool {other}"))),
    }
}

fn decision_from_submit(args: &Value, eligible: &[ModelCard]) -> Result<RouteDecision, RouterError> {
    let model = args
        .get("model")
        .and_then(|m| m.as_str())
        .ok_or_else(|| RouterError::FailClosed("submit_route missing model".into()))?
        .to_string();
    if !eligible.iter().any(|m| m.name == model) {
        return Err(RouterError::FailClosed(format!(
            "agent chose {model} not in eligible pool"
        )));
    }
    Ok(RouteDecision {
        model,
        algorithm: args
            .get("algorithm")
            .and_then(|a| a.as_str())
            .map(|s| s.to_string()),
        reason: args
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("submit_route")
            .to_string(),
        confidence: args
            .get("confidence")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.8) as f32,
        layer: "agent".into(),
        decision: "builtin".into(),
        bypass: false,
    })
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
    decision_from_submit(&v, eligible)
}

fn validate_decision(d: RouteDecision, eligible: &[ModelCard]) -> Result<RouteDecision, RouterError> {
    if !eligible.iter().any(|m| m.name == d.model) {
        return Err(RouterError::FailClosed(format!(
            "agent chose {} not in eligible pool",
            d.model
        )));
    }
    Ok(d)
}

pub fn request_view(req: &ChatRequest) -> Value {
    let chars: usize = req.messages.iter().map(|m| m.text().len()).sum();
    let has_tools = req.extra.contains_key("tools") || req.extra.contains_key("tool_choice");
    json!({
        "message_count": req.messages.len(),
        "roles": req.messages.iter().map(|m| m.role.clone()).collect::<Vec<_>>(),
        "approx_chars": chars,
        "stream": req.stream,
        "has_tools": has_tools,
    })
}

pub fn task_from(req: &ChatRequest, eligible: Vec<ModelCard>, agent: &AgentRecipe) -> RouteTask {
    let max_turns = agent
        .max_turns
        .unwrap_or(DEFAULT_MAX_TURNS)
        .clamp(1, MAX_TURNS_CLAMP);
    RouteTask {
        prompt: req.prompt_text(),
        eligible,
        system: agent
            .prompt
            .clone()
            .unwrap_or_else(|| DEFAULT_SYSTEM.into()),
        timeout_ms: agent.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
        max_turns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str) -> ModelCard {
        ModelCard {
            name: name.into(),
            locality: "local".into(),
            modality: "text".into(),
            capabilities: vec!["chat".into()],
            provider_model_id: "x".into(),
        }
    }

    #[test]
    fn reject_non_json() {
        assert!(parse_decision_json("not json", &[card("local/general")]).is_err());
    }

    #[test]
    fn reject_unknown_model() {
        assert!(parse_decision_json(r#"{"model":"cloud/x"}"#, &[card("local/general")]).is_err());
    }

    #[test]
    fn accept_json() {
        let d = parse_decision_json(
            r#"{"model":"local/general","reason":"ok","confidence":0.9}"#,
            &[card("local/general")],
        )
        .unwrap();
        assert_eq!(d.model, "local/general");
    }

    #[test]
    fn tools_list_and_health() {
        let eligible = vec![card("local/general")];
        let mut tools = ToolRuntime::default();
        tools.failures.insert("local/general".into(), 2);
        tools.latency_ms.insert("local/general".into(), 12.5);
        let listed = run_tool("list_eligible_models", &json!({}), &eligible, &tools).unwrap();
        assert_eq!(listed[0]["name"], "local/general");
        let health = run_tool("get_backend_health", &json!({}), &eligible, &tools).unwrap();
        assert_eq!(health["local/general"]["failures"], 2);
        assert_eq!(health["local/general"]["healthy"], false);
        let lat = run_tool("get_recent_latency", &json!({}), &eligible, &tools).unwrap();
        assert_eq!(lat["local/general"], 12.5);
    }

    #[test]
    fn max_turns_clamped() {
        let agent = AgentRecipe {
            max_turns: Some(99),
            timeout_ms: None,
            fallback: None,
            prompt: None,
            model: None,
            endpoint: None,
        };
        let t = task_from(
            &ChatRequest {
                model: "aria/agent-auto".into(),
                messages: vec![],
                stream: false,
                max_tokens: None,
                temperature: None,
                extra: Default::default(),
            },
            vec![card("local/general")],
            &agent,
        );
        assert_eq!(t.max_turns, MAX_TURNS_CLAMP);
    }

    #[tokio::test]
    async fn canned_and_first_eligible() {
        let eligible = vec![card("local/general")];
        let tools = ToolRuntime::default();
        let canned = BuiltinAgent {
            endpoint: None,
            model: "router-llm".into(),
            canned: Some(RouteDecision {
                model: "local/general".into(),
                algorithm: None,
                reason: "canned".into(),
                confidence: 1.0,
                layer: "agent".into(),
                decision: "builtin".into(),
                bypass: false,
            }),
        };
        let task = RouteTask {
            prompt: "hi".into(),
            eligible: eligible.clone(),
            system: DEFAULT_SYSTEM.into(),
            timeout_ms: 1000,
            max_turns: 3,
        };
        assert_eq!(canned.route(task.clone(), &tools).await.unwrap().reason, "canned");
        let first = BuiltinAgent {
            endpoint: None,
            model: "router-llm".into(),
            canned: None,
        };
        assert_eq!(
            first.route(task, &tools).await.unwrap().reason,
            "builtin:first-eligible"
        );
    }
}
