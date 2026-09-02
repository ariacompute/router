//! Route-local plugins.

use ariarouter_config::PluginRef;
use ariarouter_core::{ChatRequest, RouterError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub enum PluginOutcome {
    Continue(ChatRequest),
    FastResponse(Value),
}

pub struct PluginHost {
    cache: Mutex<HashMap<String, Value>>,
}

impl Default for PluginHost {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }
}

const IMPLEMENTED: &[&str] = &[
    "header-mutation",
    "header_mutation",
    "request-params",
    "request_params",
    "system-prompt",
    "system_prompt",
    "fast-response",
    "fast_response",
    "response-cache",
    "response_cache",
];

pub fn apply_request(
    host: &PluginHost,
    plugins: &[PluginRef],
    mut req: ChatRequest,
) -> Result<PluginOutcome, RouterError> {
    for p in plugins {
        if !IMPLEMENTED.contains(&p.name.as_str()) {
            return Err(RouterError::Unsupported(format!("plugin {}", p.name)));
        }
        match p.name.as_str() {
            "system-prompt" | "system_prompt" => {
                if let Some(text) = p.extra.get("content").and_then(|v| v.as_str()) {
                    req.messages.insert(
                        0,
                        ariarouter_core::ChatMessage {
                            role: "system".into(),
                            content: Value::String(text.to_string()),
                        },
                    );
                }
            }
            "request-params" | "request_params" => {
                if let Some(mt) = p.extra.get("max_tokens").and_then(|v| v.as_u64()) {
                    req.max_tokens = Some(mt as u32);
                }
            }
            "fast-response" | "fast_response" => {
                let msg = p
                    .extra
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("blocked");
                return Ok(PluginOutcome::FastResponse(serde_json::json!({
                    "id": "aria-fast",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": msg },
                        "finish_reason": "stop"
                    }]
                })));
            }
            "response-cache" | "response_cache" => {
                let key = cache_key(&req);
                if let Ok(map) = host.cache.lock() {
                    if let Some(hit) = map.get(&key) {
                        return Ok(PluginOutcome::FastResponse(hit.clone()));
                    }
                }
            }
            "header-mutation" | "header_mutation" => {}
            _ => {}
        }
    }
    Ok(PluginOutcome::Continue(req))
}

pub fn remember_response(host: &PluginHost, req: &ChatRequest, body: &Value) {
    if let Ok(mut map) = host.cache.lock() {
        map.insert(cache_key(req), body.clone());
    }
}

fn cache_key(req: &ChatRequest) -> String {
    format!("{}:{}", req.model, req.prompt_text())
}

pub fn extra_headers(plugins: &[PluginRef]) -> Vec<(String, String)> {
    let mut out = vec![];
    for p in plugins {
        if p.name == "header-mutation" || p.name == "header_mutation" {
            if let Some(set) = p.extra.get("set").and_then(|v| v.as_object()) {
                for (k, v) in set {
                    if let Some(s) = v.as_str() {
                        out.push((k.clone(), s.to_string()));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ariarouter_core::{ChatMessage, ChatRequest};

    fn req(text: &str) -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: serde_json::Value::String(text.into()),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn system_prompt_and_params() {
        let host = PluginHost::default();
        let plugins = vec![
            PluginRef {
                name: "system-prompt".into(),
                extra: [("content".into(), serde_json::json!("be brief"))].into(),
            },
            PluginRef {
                name: "request-params".into(),
                extra: [("max_tokens".into(), serde_json::json!(8))].into(),
            },
        ];
        match apply_request(&host, &plugins, req("hi")).unwrap() {
            PluginOutcome::Continue(r) => {
                assert_eq!(r.messages[0].role, "system");
                assert_eq!(r.max_tokens, Some(8));
            }
            _ => panic!("expected continue"),
        }
    }

    #[test]
    fn fast_response_short_circuits() {
        let host = PluginHost::default();
        let plugins = vec![PluginRef {
            name: "fast-response".into(),
            extra: [("message".into(), serde_json::json!("blocked"))].into(),
        }];
        match apply_request(&host, &plugins, req("hi")).unwrap() {
            PluginOutcome::FastResponse(v) => {
                assert_eq!(v["choices"][0]["message"]["content"], "blocked");
            }
            _ => panic!("expected fast"),
        }
    }

    #[test]
    fn response_cache_hits() {
        let host = PluginHost::default();
        let r = req("hi");
        remember_response(&host, &r, &serde_json::json!({"cached": true}));
        let plugins = vec![PluginRef {
            name: "response-cache".into(),
            extra: Default::default(),
        }];
        match apply_request(&host, &plugins, r).unwrap() {
            PluginOutcome::FastResponse(v) => assert_eq!(v["cached"], true),
            _ => panic!("expected cache hit"),
        }
    }

    #[test]
    fn header_mutation_collects() {
        let plugins = vec![PluginRef {
            name: "header-mutation".into(),
            extra: [(
                "set".into(),
                serde_json::json!({"x-test": "1"}),
            )]
            .into(),
        }];
        let h = extra_headers(&plugins);
        assert_eq!(h, vec![("x-test".into(), "1".into())]);
    }

    #[test]
    fn unknown_plugin_unsupported() {
        let host = PluginHost::default();
        let plugins = vec![PluginRef {
            name: "jailbreak-filter".into(),
            extra: Default::default(),
        }];
        assert!(matches!(
            apply_request(&host, &plugins, req("hi")),
            Err(RouterError::Unsupported(_))
        ));
    }
}
