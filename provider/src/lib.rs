//! OpenAI-compatible upstream forwarding + health/latency.

use ariarouter_config::{ProviderModel, RouterDocument};
use ariarouter_core::{ChatRequest, RouterError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Default)]
pub struct PoolState {
    pub latency_ms: Mutex<HashMap<String, f32>>,
    pub failures: Mutex<HashMap<String, u32>>,
}

impl PoolState {
    pub fn record(&self, model: &str, ms: f32, ok: bool) {
        if let Ok(mut m) = self.latency_ms.lock() {
            let prev = m.get(model).copied().unwrap_or(ms);
            m.insert(model.to_string(), prev * 0.7 + ms * 0.3);
        }
        if let Ok(mut f) = self.failures.lock() {
            if ok {
                f.insert(model.to_string(), 0);
            } else {
                *f.entry(model.to_string()).or_insert(0) += 1;
            }
        }
    }

    pub fn latency_map(&self) -> HashMap<String, f32> {
        self.latency_ms.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn failures_map(&self) -> HashMap<String, u32> {
        self.failures.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

pub async fn forward(
    doc: &RouterDocument,
    model: &str,
    req: &ChatRequest,
    extra_headers: &[(String, String)],
    pool: &PoolState,
) -> Result<Value, RouterError> {
    let provider = doc.provider(model).ok_or_else(|| {
        RouterError::FailClosed(format!("unknown provider model {model}"))
    })?;
    let backend = pick_backend(provider)?;
    let url = format!("{}/v1/chat/completions", backend.url());
    let mut body = serde_json::to_value(req).map_err(|e| RouterError::InvalidParam(e.to_string()))?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "model".into(),
            Value::String(if provider.provider_model_id.is_empty() {
                model.to_string()
            } else {
                provider.provider_model_id.clone()
            }),
        );
    }
    let mut builder = reqwest::Client::new().post(&url).json(&body);
    if let Some(key) = api_key(backend) {
        builder = builder.bearer_auth(key);
    }
    for (k, v) in extra_headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let t0 = Instant::now();
    let resp = builder.send().await.map_err(|e| {
        pool.record(model, t0.elapsed().as_millis() as f32, false);
        RouterError::Upstream(e.to_string())
    })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| RouterError::Upstream(e.to_string()))?;
    let ms = t0.elapsed().as_millis() as f32;
    if !status.is_success() {
        pool.record(model, ms, false);
        return Err(RouterError::Upstream(format!("{status}: {text}")));
    }
    pool.record(model, ms, true);
    serde_json::from_str(&text).map_err(|e| RouterError::Upstream(e.to_string()))
}

pub async fn forward_sse_text(
    doc: &RouterDocument,
    model: &str,
    req: &ChatRequest,
    extra_headers: &[(String, String)],
    pool: &PoolState,
) -> Result<String, RouterError> {
    let provider = doc.provider(model).ok_or_else(|| {
        RouterError::FailClosed(format!("unknown provider model {model}"))
    })?;
    let backend = pick_backend(provider)?;
    let url = format!("{}/v1/chat/completions", backend.url());
    let mut body = serde_json::to_value(req).map_err(|e| RouterError::InvalidParam(e.to_string()))?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "model".into(),
            Value::String(if provider.provider_model_id.is_empty() {
                model.to_string()
            } else {
                provider.provider_model_id.clone()
            }),
        );
        obj.insert("stream".into(), Value::Bool(true));
    }
    let mut builder = reqwest::Client::new().post(&url).json(&body);
    if let Some(key) = api_key(backend) {
        builder = builder.bearer_auth(key);
    }
    for (k, v) in extra_headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let t0 = Instant::now();
    let resp = builder.send().await.map_err(|e| RouterError::Upstream(e.to_string()))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| RouterError::Upstream(e.to_string()))?;
    pool.record(model, t0.elapsed().as_millis() as f32, status.is_success());
    if !status.is_success() {
        return Err(RouterError::Upstream(format!("{status}: {text}")));
    }
    Ok(text)
}

fn pick_backend(p: &ProviderModel) -> Result<&ariarouter_config::BackendRef, RouterError> {
    p.backend_refs
        .iter()
        .max_by_key(|b| b.weight)
        .ok_or_else(|| RouterError::Config(format!("model {} has no backend_refs", p.name)))
}

fn api_key(b: &ariarouter_config::BackendRef) -> Option<String> {
    if let Some(k) = &b.api_key {
        if !k.is_empty() {
            return Some(k.clone());
        }
    }
    b.api_key_env
        .as_ref()
        .and_then(|e| std::env::var(e).ok())
        .filter(|s| !s.is_empty())
}
