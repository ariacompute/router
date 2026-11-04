//! Lightweight in-process builtin agent: fixed tools + limited turns.

use aria_router_config::AgentRecipe;
use aria_router_core::{ChatRequest, ModelCard, RouteDecision, RouterError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const DEFAULT_MAX_TURNS: u32 = 3;
const MAX_TURNS_CLAMP: u32 = 8;

const DEFAULT_SYSTEM: &str = "You are the aria-router builtin agent. \
Use tools if needed, then call submit_route with one eligible model. JSON tools only. \
Pick among pool tiers when present: large = explain/how-it-works; \
mid = systems/trade-offs/multi-concept; small = pure factoids/acronyms/greetings. \
Prefer large over mid over small when intents overlap. Include a short reason.";

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
    /// Bearer token for `endpoint` (e.g. from provider `api_key_env`).
    pub api_key: Option<String>,
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
                routing_latency_ms: None,
                cache_response: false,
                retention_drop: None,
                retention_ttl_turns: None,
                retention_keep_current_model: None,
                retention_prefer_prefix: None,
                semantic_cache_hit: None,
                replay_id: None,
                session: None,
            });
        };

        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(task.timeout_ms.max(1));
        static AGENT_HTTP: OnceLock<reqwest::Client> = OnceLock::new();
        let client = AGENT_HTTP.get_or_init(reqwest::Client::new);
        let url = format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'));
        let eligible_line = task
            .eligible
            .iter()
            .map(|m| {
                format!(
                    "{} [tier={} upstream={}]",
                    m.name,
                    tier_of(m),
                    if m.provider_model_id.is_empty() {
                        "-"
                    } else {
                        m.provider_model_id.as_str()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut messages = vec![
            json!({"role": "system", "content": task.system}),
            json!({
                "role": "user",
                "content": format!(
                    "Route this request.\nEligible (also via list_eligible_models): {}\nRequest:\n{}",
                    eligible_line,
                    task.prompt
                )
            }),
        ];
        let tool_defs = tool_definitions();

        for turn in 0..task.max_turns {
            // `remaining == 0` makes the per-request timeout fire immediately.
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let body = json!({
                "model": self.model,
                "messages": messages,
                "tools": tool_defs,
                "tool_choice": "auto",
                "temperature": 0
            });
            let resp = tokio::time::timeout(remaining, {
                let mut req = client.post(&url).json(&body);
                if let Some(key) = &self.api_key {
                    req = req.bearer_auth(key);
                }
                req.send()
            })
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
                "description": "List eligible models with tier (small/mid/large/unknown) and provider_model_id (upstream). Prefer explicit tier over guessing from names.",
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
                "description": "Finalize routing with one eligible model. Choose tier by intent: large=explain/how-it-works; mid=systems/trade-offs/multi-concept; small=factoids/acronyms/greetings. Include reason naming the tier (not upstream id).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "model": { "type": "string" },
                        "reason": { "type": "string", "description": "short rationale naming large/mid/small" },
                        "confidence": { "type": "number" },
                        "algorithm": { "type": "string" }
                    },
                    "required": ["model"]
                }
            }
        }
    ])
}

/// Prefer explicit `ModelCard.tier`; fall back to logical-name suffix heuristics.
fn tier_of(card: &ModelCard) -> &'static str {
    if let Some(t) = card.tier.as_deref() {
        match t.trim().to_ascii_lowercase().as_str() {
            "small" => return "small",
            "mid" | "medium" => return "mid",
            "large" => return "large",
            _ => {}
        }
    }
    tier_hint_from_name(&card.name)
}

/// Pool-role hint from logical name suffix (not user-prompt keyword routing).
fn tier_hint_from_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with("-large") || lower.contains("/ariamodel-large") || lower.ends_with("/large")
    {
        "large"
    } else if lower.ends_with("-mid") || lower.contains("/ariamodel-mid") || lower.ends_with("/mid")
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
                "tier": tier_of(m),
                "provider_model_id": m.provider_model_id,
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

fn decision_from_submit(
    args: &Value,
    eligible: &[ModelCard],
) -> Result<RouteDecision, RouterError> {
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
        routing_latency_ms: None,
        cache_response: false,
        retention_drop: None,
        retention_ttl_turns: None,
        retention_keep_current_model: None,
        retention_prefer_prefix: None,
        semantic_cache_hit: None,
        replay_id: None,
        session: None,
    })
}

pub fn parse_decision_json(
    raw: &str,
    eligible: &[ModelCard],
) -> Result<RouteDecision, RouterError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let v: Value = serde_json::from_str(trimmed)
        .map_err(|_| RouterError::FailClosed(format!("agent output is not JSON: {trimmed}")))?;
    decision_from_submit(&v, eligible)
}

fn validate_decision(
    d: RouteDecision,
    eligible: &[ModelCard],
) -> Result<RouteDecision, RouterError> {
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
            tier: None,
        }
    }

    fn card_tier(name: &str, tier: &str, upstream: &str) -> ModelCard {
        ModelCard {
            name: name.into(),
            locality: "cloud".into(),
            modality: "text".into(),
            capabilities: vec!["chat".into()],
            provider_model_id: upstream.into(),
            tier: Some(tier.into()),
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
        assert_eq!(listed[0]["tier"], "unknown");
        assert_eq!(listed[0]["provider_model_id"], "x");
        let health = run_tool("get_backend_health", &json!({}), &eligible, &tools).unwrap();
        assert_eq!(health["local/general"]["failures"], 2);
        assert_eq!(health["local/general"]["healthy"], false);
        let lat = run_tool("get_recent_latency", &json!({}), &eligible, &tools).unwrap();
        assert_eq!(lat["local/general"], 12.5);
    }

    #[test]
    fn tier_hint_from_logical_suffix() {
        assert_eq!(tier_hint_from_name("ariacompute/ariamodel-small"), "small");
        assert_eq!(tier_hint_from_name("ariacompute/ariamodel-mid"), "mid");
        assert_eq!(tier_hint_from_name("ariacompute/ariamodel-large"), "large");
        assert_eq!(tier_hint_from_name("local/general"), "unknown");
        let eligible = vec![
            card("ariacompute/ariamodel-small"),
            card("ariacompute/ariamodel-mid"),
            card("ariacompute/ariamodel-large"),
        ];
        let listed = run_tool(
            "list_eligible_models",
            &json!({}),
            &eligible,
            &ToolRuntime::default(),
        )
        .unwrap();
        assert_eq!(listed[0]["tier"], "small");
        assert_eq!(listed[1]["tier"], "mid");
        assert_eq!(listed[2]["tier"], "large");
    }

    #[test]
    fn tier_prefers_explicit_over_name() {
        let custom = card_tier("acme/foo", "large", "other-model");
        assert_eq!(tier_of(&custom), "large");
        assert_eq!(tier_hint_from_name(&custom.name), "unknown");
        let listed = run_tool(
            "list_eligible_models",
            &json!({}),
            &[custom],
            &ToolRuntime::default(),
        )
        .unwrap();
        assert_eq!(listed[0]["tier"], "large");
        assert_eq!(listed[0]["provider_model_id"], "other-model");
    }

    #[test]
    fn upstream_change_keeps_tier() {
        let mut c = card("ariacompute/ariamodel-mid");
        c.tier = Some("mid".into());
        c.provider_model_id = "totally-different-upstream".into();
        assert_eq!(tier_of(&c), "mid");
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
            prefix_reuse: None,
            decision_ext: None,
            delegate_endpoint: None,
            worker_plan: None,
            delegate_auth: None,
        };
        let t = task_from(
            &ChatRequest {
                model: "ariacompute/agent-auto".into(),
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
            api_key: None,
            canned: Some(RouteDecision {
                model: "local/general".into(),
                algorithm: None,
                reason: "canned".into(),
                confidence: 1.0,
                layer: "agent".into(),
                decision: "builtin".into(),
                bypass: false,
                routing_latency_ms: None,
                cache_response: false,
                retention_drop: None,
                retention_ttl_turns: None,
                retention_keep_current_model: None,
                retention_prefer_prefix: None,
                semantic_cache_hit: None,
                replay_id: None,
                session: None,
            }),
        };
        let task = RouteTask {
            prompt: "hi".into(),
            eligible: eligible.clone(),
            system: DEFAULT_SYSTEM.into(),
            timeout_ms: 1000,
            max_turns: 3,
        };
        assert_eq!(
            canned.route(task.clone(), &tools).await.unwrap().reason,
            "canned"
        );
        let first = BuiltinAgent {
            endpoint: None,
            model: "router-llm".into(),
            api_key: None,
            canned: None,
        };
        assert_eq!(
            first.route(task.clone(), &tools).await.unwrap().reason,
            "builtin:first-eligible"
        );
        // Canned decision outside the pool fails closed.
        let canned_bad = BuiltinAgent {
            endpoint: None,
            model: "router-llm".into(),
            api_key: None,
            canned: Some(RouteDecision::bypass("cloud/not-eligible")),
        };
        let err = canned_bad
            .route(task.clone(), &tools)
            .await
            .expect_err("canned outside pool");
        assert!(matches!(err, RouterError::FailClosed(_)));
    }

    // ---- loopback LLM stub -------------------------------------------------

    struct StubLlm {
        endpoint: String,
        seen_auth: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        seen_bodies: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl StubLlm {
        async fn start(responses: Vec<serde_json::Value>) -> Self {
            use axum::extract::Json;
            use axum::http::HeaderMap;
            use std::collections::VecDeque;
            use std::sync::{Arc, Mutex};

            let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
            let seen_auth = Arc::new(Mutex::new(Vec::new()));
            let seen_bodies = Arc::new(Mutex::new(Vec::new()));
            let (q, auth, bodies) = (queue.clone(), seen_auth.clone(), seen_bodies.clone());

            let app = axum::Router::new().route(
                "/v1/chat/completions",
                axum::routing::post(
                    move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                        let (q, auth, bodies) = (q.clone(), auth.clone(), bodies.clone());
                        async move {
                            auth.lock().unwrap().push(
                                headers
                                    .get("authorization")
                                    .and_then(|h| h.to_str().ok())
                                    .unwrap_or("")
                                    .to_string(),
                            );
                            bodies.lock().unwrap().push(body);
                            let next = {
                                let mut g = q.lock().unwrap();
                                if g.len() > 1 {
                                    g.pop_front().unwrap()
                                } else {
                                    g.front().cloned().unwrap_or(serde_json::json!({}))
                                }
                            };
                            match next.get("__status").and_then(|s| s.as_u64()) {
                                Some(code) => (
                                    axum::http::StatusCode::from_u16(code as u16).unwrap(),
                                    "stub error",
                                )
                                    .into_response(),
                                None => {
                                    // The `__status` marker is not part of the OpenAI body.
                                    let mut next = next;
                                    if let Some(obj) = next.as_object_mut() {
                                        obj.remove("__status");
                                    }
                                    axum::Json(next).into_response()
                                }
                            }
                        }
                    },
                ),
            );

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let handle = tokio::spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = rx.await;
                    })
                    .await
                    .unwrap();
            });
            Self {
                endpoint: format!("http://{addr}"),
                seen_auth,
                seen_bodies,
                stop: Some(tx),
                handle: Some(handle),
            }
        }

        async fn shutdown(mut self) {
            let tx = self.stop.take().expect("stop sender");
            let handle = self.handle.take().expect("handle");
            let _ = tx.send(());
            let _ = handle.await;
        }
    }

    use axum::response::IntoResponse;

    fn msg_tool(id: &str, name: &str, args: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": args.to_string()}
                    }]
                }
            }]
        })
    }

    fn msg_json(decision: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [{"message": {"role": "assistant", "content": decision.to_string()}}]
        })
    }

    fn agent_with(endpoint: &str, api_key: Option<&str>) -> BuiltinAgent {
        BuiltinAgent {
            endpoint: Some(endpoint.to_string()),
            model: "router-llm".into(),
            api_key: api_key.map(str::to_string),
            canned: None,
        }
    }

    fn task_for(eligible: Vec<ModelCard>, max_turns: u32) -> RouteTask {
        RouteTask {
            prompt: "explain rust ownership".into(),
            eligible,
            system: DEFAULT_SYSTEM.into(),
            timeout_ms: 2000,
            max_turns,
        }
    }

    #[tokio::test]
    async fn empty_pool_fails_closed() {
        let err = BuiltinAgent {
            endpoint: None,
            model: "m".into(),
            api_key: None,
            canned: None,
        }
        .route(task_for(vec![], 1), &ToolRuntime::default())
        .await
        .expect_err("no eligible");
        assert!(matches!(err, RouterError::FailClosed(_)));
    }

    #[tokio::test]
    async fn tool_loop_then_submit_route() {
        let stub = StubLlm::start(vec![
            msg_tool("c1", "list_eligible_models", json!({})),
            msg_tool(
                "c2",
                "submit_route",
                json!({"model": "local/b", "reason": "mid tier", "confidence": 0.7, "algorithm": "static"}),
            ),
        ])
        .await;
        let agent = agent_with(&stub.endpoint, Some("sk-agent"));
        let mut no_upstream = card("local/c");
        no_upstream.provider_model_id.clear();
        let task = task_for(
            vec![
                card("local/a"),
                card_tier("local/b", "mid", "up-x"),
                no_upstream,
            ],
            3,
        );
        let d = agent.route(task, &ToolRuntime::default()).await.unwrap();
        assert_eq!(d.model, "local/b");
        assert_eq!(d.reason, "mid tier");
        assert!((d.confidence - 0.7).abs() < 1e-6);
        assert_eq!(d.algorithm.as_deref(), Some("static"));
        // api_key became a bearer token and the tool defs were advertised.
        let auth = stub.seen_auth.lock().unwrap()[0].clone();
        assert_eq!(auth, "Bearer sk-agent");
        {
            let bodies = stub.seen_bodies.lock().unwrap();
            assert_eq!(bodies[0]["tools"].as_array().unwrap().len(), 5);
            let line = bodies[0]["messages"][1]["content"].as_str().unwrap();
            assert!(line.contains("local/b [tier=mid upstream=up-x]"), "{line}");
            // Models without an upstream id render as `-`.
            assert!(line.contains("local/c [tier=unknown upstream=-]"), "{line}");
        }
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn non_terminal_tools_then_content_decision() {
        let stub = StubLlm::start(vec![
            msg_tool("c1", "get_backend_health", json!({"model": "local/a"})),
            msg_tool("c2", "get_recent_latency", json!({})),
            msg_tool("c3", "get_request_view", json!({})),
            json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "```json\n{\"model\":\"local/a\",\"reason\":\"small\"}\n```"
                    }
                }]
            }),
        ])
        .await;
        let mut tools = ToolRuntime::default();
        tools.failures.insert("local/a".into(), 1);
        tools.latency_ms.insert("local/a".into(), 7.0);
        tools.request_view = json!({"message_count": 1});
        let d = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 4), &tools)
            .await
            .unwrap();
        assert_eq!(d.model, "local/a");
        assert_eq!(d.reason, "small");
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn exceeding_max_turns_fails_closed() {
        let stub = StubLlm::start(vec![msg_tool("c1", "list_eligible_models", json!({}))]).await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 1), &ToolRuntime::default())
            .await
            .expect_err("max turns");
        assert!(matches!(err, RouterError::FailClosed(_)));
        assert!(err.to_string().contains("exceeded max_turns"));
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn zero_turn_budget_fails_closed() {
        let stub = StubLlm::start(vec![msg_json(json!({"model": "local/a"}))]).await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 0), &ToolRuntime::default())
            .await
            .expect_err("zero turns");
        assert!(matches!(err, RouterError::FailClosed(_)));
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn upstream_error_status_and_bad_body() {
        let stub = StubLlm::start(vec![json!({"__status": 500})]).await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 1), &ToolRuntime::default())
            .await
            .expect_err("upstream 500");
        assert!(matches!(err, RouterError::Extension(_)));
        stub.shutdown().await;

        let stub = StubLlm::start(vec![json!({"choices": []})]).await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 1), &ToolRuntime::default())
            .await
            .expect_err("missing message");
        assert!(matches!(err, RouterError::FailClosed(_)));
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn empty_tool_calls_fall_through_to_content() {
        let stub = StubLlm::start(vec![json!({
            "choices": [{
                "message": {"role": "assistant", "content": "{\"model\":\"local/a\"}", "tool_calls": []}
            }]
        })])
        .await;
        let d = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 1), &ToolRuntime::default())
            .await
            .unwrap();
        assert_eq!(d.reason, "submit_route");
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn empty_message_fails_closed() {
        let stub = StubLlm::start(vec![json!({
            "choices": [{"message": {"role": "assistant"}}]
        })])
        .await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 1), &ToolRuntime::default())
            .await
            .expect_err("empty message");
        assert!(matches!(err, RouterError::FailClosed(_)));
        assert!(err
            .to_string()
            .contains("produced no tool_calls or content"));
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn unknown_tool_and_invalid_arguments() {
        // Invalid JSON arguments fall back to `{}`; unknown tool then errors.
        let stub = StubLlm::start(vec![json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{"id": "c1", "function": {"name": "bogus_tool", "arguments": "{not json"}}]
                }
            }]
        })])
        .await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 2), &ToolRuntime::default())
            .await
            .expect_err("unknown tool");
        assert!(matches!(err, RouterError::Unsupported(_)), "{err}");
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn submit_route_rejects_model_outside_pool() {
        let stub = StubLlm::start(vec![msg_tool(
            "c1",
            "submit_route",
            json!({"model": "cloud/ghost"}),
        )])
        .await;
        let err = agent_with(&stub.endpoint, None)
            .route(task_for(vec![card("local/a")], 2), &ToolRuntime::default())
            .await
            .expect_err("outside pool");
        assert!(matches!(err, RouterError::FailClosed(_)));
        stub.shutdown().await;
    }

    #[tokio::test]
    async fn llm_timeout_is_reported() {
        // Unroutable port: the send fails fast with an extension error.
        let err = agent_with("http://127.0.0.1:1", None)
            .route(task_for(vec![card("local/a")], 1), &ToolRuntime::default())
            .await
            .expect_err("connection refused");
        assert!(matches!(err, RouterError::Extension(_)), "{err}");
    }

    #[test]
    fn tool_runtime_edge_cases() {
        let eligible = vec![card("local/a"), card("local/b")];
        let mut tools = ToolRuntime::default();
        tools.failures.insert("local/b".into(), 3);
        // Filters skip non-matching models.
        let health = run_tool(
            "get_backend_health",
            &json!({"model": "local/a"}),
            &eligible,
            &tools,
        )
        .unwrap();
        assert!(health.get("local/b").is_none());
        assert_eq!(health["local/a"]["healthy"], true);
        let lat = run_tool(
            "get_recent_latency",
            &json!({"model": "local/b"}),
            &eligible,
            &tools,
        )
        .unwrap();
        assert!(lat.as_object().unwrap().is_empty());

        let view = run_tool(
            "get_request_view",
            &json!({}),
            &eligible,
            &ToolRuntime {
                request_view: json!({"k": 1}),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(view["k"], 1);

        // submit_route is terminal only; direct calls are rejected.
        assert!(matches!(
            run_tool("submit_route", &json!({}), &eligible, &tools),
            Err(RouterError::InvalidParam(_))
        ));
        assert!(matches!(
            run_tool("nope", &json!({}), &eligible, &tools),
            Err(RouterError::Unsupported(_))
        ));
    }

    #[test]
    fn tier_of_unknown_explicit_tier_uses_name() {
        let card = card_tier("acme/thing-large", "weird", "up");
        assert_eq!(tier_of(&card), "large");
        assert_eq!(tier_hint_from_name("svc/x/large"), "large");
        assert_eq!(tier_hint_from_name("x/large"), "large");
        assert_eq!(tier_hint_from_name("x-MID"), "mid");
        assert_eq!(tier_hint_from_name("x/mid"), "mid");
        assert_eq!(tier_hint_from_name("x-small"), "small");
        assert_eq!(tier_hint_from_name("x/small"), "small");
        assert_eq!(tier_hint_from_name("x"), "unknown");
    }
}
