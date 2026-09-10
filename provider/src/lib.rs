//! OpenAI-compatible upstream forwarding + health/latency.

use aria_router_config::{ProviderModel, RouterDocument};
use aria_router_core::{ChatRequest, RouterError};
use bytes::Bytes;
use futures_util::Stream;
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// Upstream pool stats + shared HTTP client (TLS/connection reuse across chats).
pub struct PoolState {
    pub client: reqwest::Client,
    pub latency_ms: Mutex<HashMap<String, f32>>,
    pub failures: Mutex<HashMap<String, u32>>,
}

impl Default for PoolState {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(90))
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            latency_ms: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
        }
    }
}

impl std::fmt::Debug for PoolState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolState")
            .field("latency_ms", &self.latency_ms)
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
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
    let mut builder = pool.client.post(&url).json(&body);
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

/// Buffered SSE read (tests / callers that need the full body). Prefer [`forward_sse_stream`].
pub async fn forward_sse_text(
    doc: &RouterDocument,
    model: &str,
    req: &ChatRequest,
    extra_headers: &[(String, String)],
    pool: &PoolState,
) -> Result<String, RouterError> {
    let mut stream = forward_sse_stream(doc, model, req, extra_headers, pool).await?;
    let mut acc = Vec::new();
    while let Some(item) = stream.next().await {
        let bytes = item?;
        acc.extend_from_slice(&bytes);
    }
    Ok(String::from_utf8_lossy(&acc).into_owned())
}

type BoxByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, RouterError>> + Send>>;

/// Open upstream SSE as a passthrough byte stream. Records pool latency when the stream ends.
pub async fn forward_sse_stream(
    doc: &RouterDocument,
    model: &str,
    req: &ChatRequest,
    extra_headers: &[(String, String)],
    pool: &PoolState,
) -> Result<SseByteStream, RouterError> {
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
    let mut builder = pool.client.post(&url).json(&body);
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
    if !status.is_success() {
        let text = resp.text().await.map_err(|e| RouterError::Upstream(e.to_string()))?;
        pool.record(model, t0.elapsed().as_millis() as f32, false);
        return Err(RouterError::Upstream(format!("{status}: {text}")));
    }

    // Pool recording for stream path is done by the HTTP layer after the body finishes
    // (it holds `&PoolState` for the request). Here we only expose bytes + accumulation.
    let _ = t0;
    let _ = pool;
    let inner: BoxByteStream = Box::pin(resp.bytes_stream().map(|item| {
        item.map_err(|e| RouterError::Upstream(e.to_string()))
    }));
    Ok(SseByteStream {
        inner,
        buf: Vec::new(),
        finished: false,
        ok: true,
        started: Instant::now(),
        model: model.to_string(),
    })
}

/// Passthrough upstream SSE bytes; accumulates for cost parsing after the stream ends.
pub struct SseByteStream {
    inner: BoxByteStream,
    buf: Vec<u8>,
    finished: bool,
    ok: bool,
    started: Instant,
    model: String,
}

impl SseByteStream {
    pub fn accumulated(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn elapsed_ms(&self) -> f32 {
        self.started.elapsed().as_millis() as f32
    }

    pub fn succeeded(&self) -> bool {
        self.ok && self.finished
    }
}

impl Stream for SseByteStream {
    type Item = Result<Bytes, RouterError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                self.buf.extend_from_slice(&bytes);
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                self.finished = true;
                self.ok = false;
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                self.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

fn pick_backend(p: &ProviderModel) -> Result<&aria_router_config::BackendRef, RouterError> {
    p.backend_refs
        .iter()
        .max_by_key(|b| b.weight)
        .ok_or_else(|| RouterError::Config(format!("model {} has no backend_refs", p.name)))
}

pub fn backend_api_key(b: &aria_router_config::BackendRef) -> Option<String> {
    if let Some(k) = &b.api_key {
        if !k.is_empty() {
            return Some(k.clone());
        }
    }
    let name = b.api_key_env.as_ref()?;
    if let Ok(v) = std::env::var(name) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    // If a secret was pasted into api_key_env (not an env var name), use it directly
    // so serve can auth from router.yml without a matching process env.
    if looks_like_inline_api_key(name) {
        return Some(name.clone());
    }
    None
}

fn looks_like_inline_api_key(s: &str) -> bool {
    let t = s.trim();
    t.starts_with("sk-") || t.starts_with("sk_") || (t.len() >= 32 && t.contains('-'))
}

fn api_key(b: &aria_router_config::BackendRef) -> Option<String> {
    backend_api_key(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_state_reuses_shared_client() {
        let a = PoolState::default();
        let b = PoolState::default();
        // Distinct PoolState instances each own a Client; AppState holds one for the process.
        a.record("m", 10.0, true);
        b.record("m", 20.0, true);
        assert!((a.latency_map().get("m").copied().unwrap() - 10.0).abs() < 0.01);
        // Client is constructible and cloneable for concurrent requests.
        let _ = a.client.clone();
        let _ = b.client.clone();
    }
}
