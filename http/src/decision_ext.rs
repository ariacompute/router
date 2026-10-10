//! Decision-model extension point.
//!
//! A per-recipe `decision_ext` lets an external HTTP decision service override the
//! model choice for semantic / agent routing. The contract mirrors `afm-d` System
//! One: `POST {endpoint}/v1/decision` with the `ChatRequest` body, returning a
//! JSON `{ "model": "...", "algorithm"?, "reason"?, "confidence"?, "decision"? }`.
//!
//! Unreachable / timeout / non-success / malformed responses fall back to the
//! configured `fallback` model, or fail closed (`RouterError`) when no fallback is
//! set. This is a `per-recipe` extension point — it does NOT introduce a top-level
//! `extensions` block (the repo deliberately rejects that).

use aria_router_config::DecisionExtCfg;
use aria_router_core::{ChatRequest, RouteDecision, RouterError};
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_TIMEOUT_MS: u64 = 5000;

/// Call the external decision service and translate the response into a route decision.
pub async fn decide(req: &ChatRequest, cfg: &DecisionExtCfg) -> Result<RouteDecision, RouterError> {
    let endpoint = cfg
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = s.trim_end_matches('/');
            if s.starts_with("http://") || s.starts_with("https://") {
                s.to_string()
            } else {
                format!("http://{s}")
            }
        });
    let endpoint = match endpoint {
        Some(e) => e,
        None => {
            return apply_fallback_or_err(
                cfg.fallback.as_deref(),
                RouterError::Unsupported("decision_ext has no endpoint".into()),
            );
        }
    };
    let url = format!("{endpoint}/v1/decision");
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(reqwest::Client::new);
    let timeout = Duration::from_millis(cfg.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1));
    let mut req_builder = client.post(&url).json(req);
    if let Some(auth) = cfg.auth.as_deref() {
        if !auth.trim().is_empty() {
            req_builder = req_builder.bearer_auth(auth);
        }
    }
    let send = tokio::time::timeout(timeout, req_builder.send()).await;
    let resp = match send {
        Err(_) => {
            return apply_fallback_or_err(
                cfg.fallback.as_deref(),
                RouterError::Timeout("decision_ext".into()),
            );
        }
        Ok(Err(e)) => {
            return apply_fallback_or_err(
                cfg.fallback.as_deref(),
                RouterError::Extension(format!("decision_ext: {e}")),
            );
        }
        Ok(Ok(r)) => r,
    };
    if !resp.status().is_success() {
        return apply_fallback_or_err(
            cfg.fallback.as_deref(),
            RouterError::Extension(format!("decision_ext status {}", resp.status())),
        );
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| RouterError::Extension(format!("decision_ext decode: {e}")))?;
    let model = v.get("model").and_then(|m| m.as_str()).map(String::from);
    let model = match model {
        Some(m) => m,
        None => {
            return apply_fallback_or_err(
                cfg.fallback.as_deref(),
                RouterError::Extension("decision_ext response missing `model`".into()),
            );
        }
    };
    let algorithm = v
        .get("algorithm")
        .and_then(|a| a.as_str())
        .map(String::from);
    let reason = v
        .get("reason")
        .and_then(|r| r.as_str())
        .map(String::from)
        .unwrap_or_else(|| "decision_ext".into());
    let decision = v
        .get("decision")
        .and_then(|d| d.as_str())
        .map(String::from)
        .unwrap_or_else(|| "decision_ext".into());
    let confidence = v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
    Ok(RouteDecision {
        model,
        algorithm,
        reason,
        confidence,
        layer: "decision_ext".into(),
        decision,
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

fn apply_fallback_or_err(
    fallback: Option<&str>,
    err: RouterError,
) -> Result<RouteDecision, RouterError> {
    match fallback {
        Some(f) if !f.trim().is_empty() => Ok(RouteDecision::bypass(f)),
        _ => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_core::ChatRequest;

    fn req() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: vec![],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        }
    }

    #[tokio::test]
    async fn missing_endpoint_without_fallback_errors() {
        let cfg = DecisionExtCfg::default();
        let r = decide(&req(), &cfg).await;
        assert!(matches!(r, Err(RouterError::Unsupported(_))));
    }

    #[tokio::test]
    async fn missing_endpoint_uses_fallback() {
        let cfg = DecisionExtCfg {
            fallback: Some("local/fallback".into()),
            ..Default::default()
        };
        let d = decide(&req(), &cfg).await.unwrap();
        assert_eq!(d.model, "local/fallback");
        assert!(d.bypass);
    }

    #[tokio::test]
    async fn decision_ext_happy_path_uses_service_model() {
        use axum::extract::Json;
        use axum::response::IntoResponse;
        use axum::routing::post;
        async fn ext(Json(_v): Json<serde_json::Value>) -> impl IntoResponse {
            axum::Json(serde_json::json!({
                "model": "local/override",
                "reason": "from-ext",
                "confidence": 0.9
            }))
        }
        let app = axum::Router::new().route("/v1/decision", post(ext));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let endpoint = format!("http://127.0.0.1:{}", addr.port());
        let cfg = DecisionExtCfg {
            endpoint: Some(endpoint),
            ..Default::default()
        };
        let d = decide(&req(), &cfg).await.unwrap();
        assert_eq!(d.model, "local/override");
        assert!(!d.bypass);
        assert_eq!(d.reason, "from-ext");
        assert!((d.confidence - 0.9).abs() < 1e-6);
    }

    #[tokio::test]
    async fn decision_ext_missing_model_uses_fallback() {
        use axum::extract::Json;
        use axum::response::IntoResponse;
        use axum::routing::post;
        // Response omits the required `model` field.
        async fn ext(Json(_v): Json<serde_json::Value>) -> impl IntoResponse {
            axum::Json(serde_json::json!({"reason": "oops"}))
        }
        let app = axum::Router::new().route("/v1/decision", post(ext));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let endpoint = format!("http://127.0.0.1:{}", addr.port());
        let cfg = DecisionExtCfg {
            endpoint: Some(endpoint),
            fallback: Some("local/fallback".into()),
            ..Default::default()
        };
        let d = decide(&req(), &cfg).await.unwrap();
        assert_eq!(d.model, "local/fallback");
        assert!(d.bypass);
    }
}
