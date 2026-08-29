//! Native Rust SDK for aria-router (does not dlopen `libaria_router_ffi`).

use aria_router_config::RouterDocument;
use aria_router_core::RouterError;
use aria_router_http::{data_router, last_route_json, AppState};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

pub struct Router {
    state: Option<Arc<AppState>>,
    base_url: Option<String>,
    rt: tokio::runtime::Runtime,
    auth: AuthConfig,
}

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Default)]
pub struct AuthUpdates {
    pub base_url: Option<String>,
    pub token: Option<String>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            state: None,
            base_url: None,
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("rt"),
            auth: AuthConfig::default(),
        }
    }

    pub fn auth(&mut self, u: &AuthUpdates) -> Result<&mut Self, RouterError> {
        if let Some(v) = &u.base_url {
            self.auth.base_url = v.clone();
        }
        if let Some(v) = &u.token {
            self.auth.token = v.clone();
        }
        Ok(self)
    }

    pub fn auth_status(&self) -> &AuthConfig {
        &self.auth
    }

    pub fn auth_clear(&mut self) -> &mut Self {
        self.auth = AuthConfig::default();
        self
    }

    pub fn init(&mut self, config_path: &str) -> Result<&mut Self, RouterError> {
        let doc = RouterDocument::load_path(config_path)?;
        self.state = Some(Arc::new(AppState::new(doc)));
        self.base_url = None;
        Ok(self)
    }

    pub fn connect(&mut self, base_url: &str) -> &mut Self {
        self.base_url = Some(base_url.to_string());
        self.state = None;
        self
    }

    pub fn complete(&self, messages: Value, options: Value) -> Result<Value, RouterError> {
        let model = options
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("aria/semantic-auto");
        let req = serde_json::json!({"model": model, "messages": messages});
        if let Some(st) = &self.state {
            let app = data_router(st.clone());
            let body = serde_json::to_vec(&req).unwrap();
            let resp = self
                .rt
                .block_on(async {
                    app.oneshot(
                        Request::post("/v1/chat/completions")
                            .header("content-type", "application/json")
                            .body(Body::from(body))
                            .unwrap(),
                    )
                    .await
                })
                .map_err(|e| RouterError::Upstream(e.to_string()))?;
            let bytes = self
                .rt
                .block_on(to_bytes(resp.into_body(), 1 << 22))
                .map_err(|e| RouterError::Upstream(e.to_string()))?;
            serde_json::from_slice(&bytes).map_err(|e| RouterError::Upstream(e.to_string()))
        } else {
            let url = self
                .base_url
                .as_deref()
                .or(if self.auth.base_url.is_empty() {
                    None
                } else {
                    Some(self.auth.base_url.as_str())
                })
                .ok_or_else(|| RouterError::InvalidParam("not initialized".into()))?;
            let text = self
                .rt
                .block_on(async {
                    reqwest::Client::new()
                        .post(format!("{}/v1/chat/completions", url.trim_end_matches('/')))
                        .json(&req)
                        .send()
                        .await?
                        .text()
                        .await
                })
                .map_err(|e| RouterError::Upstream(e.to_string()))?;
            serde_json::from_str(&text).map_err(|e| RouterError::Upstream(e.to_string()))
        }
    }

    pub fn models(&self) -> Result<Value, RouterError> {
        let Some(st) = &self.state else {
            return Ok(serde_json::json!({"data": []}));
        };
        let doc = st.doc.lock().unwrap();
        let data: Vec<_> = doc
            .entrypoints
            .iter()
            .flat_map(|e| e.model_names.iter().cloned())
            .collect();
        Ok(serde_json::json!({"data": data}))
    }

    pub fn last_route(&self) -> Value {
        self.state
            .as_ref()
            .map(|s| last_route_json(s))
            .unwrap_or(serde_json::json!({}))
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_models() {
        let mut r = Router::new();
        let p1 = std::path::Path::new("../../config/examples/semantic-tiny.yaml");
        let p2 = std::path::Path::new("config/examples/semantic-tiny.yaml");
        let path = if p1.exists() {
            p1
        } else {
            p2
        };
        r.init(path.to_str().unwrap()).unwrap();
        let m = r.models().unwrap();
        assert!(m["data"].as_array().unwrap().iter().any(|x| x == "aria/semantic-auto"));
    }

    #[test]
    fn auth_memory_only() {
        let mut r = Router::new();
        r.auth(&AuthUpdates {
            base_url: Some("http://127.0.0.1:8899".into()),
            token: Some("t".into()),
        })
        .unwrap();
        assert_eq!(r.auth_status().token, "t");
        r.auth_clear();
        assert!(r.auth_status().token.is_empty());
    }
}
