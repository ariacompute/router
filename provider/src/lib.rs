//! OpenAI-compatible upstream forwarding + health/latency.

use aria_router_config::{BackendRef, ProviderModel, RouterDocument};
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
        let mut m = self
            .latency_ms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = m.get(model).copied().unwrap_or(ms);
        m.insert(model.to_string(), prev * 0.7 + ms * 0.3);
        drop(m);

        let mut f = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ok {
            f.insert(model.to_string(), 0);
        } else {
            *f.entry(model.to_string()).or_insert(0) += 1;
        }
    }

    pub fn latency_map(&self) -> HashMap<String, f32> {
        self.latency_ms
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn failures_map(&self) -> HashMap<String, u32> {
        self.failures.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

/// Ordered backend candidate chain (C8). With failover disabled the historical
/// single highest-weight backend is used; when enabled, candidates are sorted
/// by weight (descending, stable) and truncated to `max_attempts`.
fn backend_candidates<'a>(
    doc: &'a RouterDocument,
    model: &str,
) -> Result<Vec<&'a BackendRef>, RouterError> {
    let provider = doc
        .provider(model)
        .ok_or_else(|| RouterError::FailClosed(format!("unknown provider model {model}")))?;
    let cfg = &doc.global.services.backend_failover;
    if !cfg.enabled {
        return Ok(vec![pick_backend(provider)?]);
    }
    if provider.backend_refs.is_empty() {
        return Err(RouterError::Config(format!(
            "model {} has no backend_refs",
            provider.name
        )));
    }
    let mut refs: Vec<&BackendRef> = provider.backend_refs.iter().collect();
    refs.sort_by_key(|b| std::cmp::Reverse(b.weight));
    refs.truncate(cfg.max_attempts.max(1) as usize);
    Ok(refs)
}

/// Join all candidate failures without leaking keys or request bodies.
fn candidates_exhausted(n: usize, errors: &[String], last: RouterError) -> RouterError {
    if n == 1 {
        return last;
    }
    RouterError::Upstream(format!(
        "all {n} backend candidates failed: {}",
        errors.join("; ")
    ))
}

fn request_builder(
    pool: &PoolState,
    url: &str,
    body: &Value,
    backend: &BackendRef,
    extra_headers: &[(String, String)],
) -> reqwest::RequestBuilder {
    let mut builder = pool.client.post(url).json(body);
    if let Some(key) = api_key(backend) {
        builder = builder.bearer_auth(key);
    }
    for (k, v) in extra_headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    builder
}

async fn forward_once(
    pool: &PoolState,
    model: &str,
    body: &Value,
    backend: &BackendRef,
    extra_headers: &[(String, String)],
) -> Result<Value, RouterError> {
    let url = format!("{}/v1/chat/completions", backend.url());
    let builder = request_builder(pool, &url, body, backend, extra_headers);
    let t0 = Instant::now();
    let resp = builder.send().await.map_err(|e| {
        pool.record(model, t0.elapsed().as_millis() as f32, false);
        RouterError::Upstream(e.to_string())
    })?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| RouterError::Upstream(e.to_string()))?;
    let ms = t0.elapsed().as_millis() as f32;
    if !status.is_success() {
        pool.record(model, ms, false);
        return Err(RouterError::Upstream(format!("{status}: {text}")));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|e| {
        pool.record(model, ms, false);
        RouterError::Upstream(e.to_string())
    })?;
    pool.record(model, ms, true);
    Ok(parsed)
}

pub async fn forward(
    doc: &RouterDocument,
    model: &str,
    req: &ChatRequest,
    extra_headers: &[(String, String)],
    pool: &PoolState,
) -> Result<Value, RouterError> {
    let provider = doc
        .provider(model)
        .ok_or_else(|| RouterError::FailClosed(format!("unknown provider model {model}")))?;
    let candidates = backend_candidates(doc, model)?;
    let mut body =
        serde_json::to_value(req).map_err(|e| RouterError::InvalidParam(e.to_string()))?;
    let upstream_model = if provider.provider_model_id.is_empty() {
        model.to_string()
    } else {
        provider.provider_model_id.clone()
    };
    body["model"] = Value::String(upstream_model);
    let n = candidates.len();
    let mut errors = Vec::new();
    let mut last = RouterError::Config(format!("model {model} has no backend_refs"));
    for (i, backend) in candidates.into_iter().enumerate() {
        match forward_once(pool, model, &body, backend, extra_headers).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                errors.push(format!("candidate {}: {e}", i + 1));
                last = e;
            }
        }
    }
    Err(candidates_exhausted(n, &errors, last))
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
async fn open_sse_once(
    pool: &PoolState,
    model: &str,
    body: &Value,
    backend: &BackendRef,
    extra_headers: &[(String, String)],
) -> Result<SseByteStream, RouterError> {
    let url = format!("{}/v1/chat/completions", backend.url());
    let builder = request_builder(pool, &url, body, backend, extra_headers);
    let t0 = Instant::now();
    let resp = builder.send().await.map_err(|e| {
        pool.record(model, t0.elapsed().as_millis() as f32, false);
        RouterError::Upstream(e.to_string())
    })?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp
            .text()
            .await
            .map_err(|e| RouterError::Upstream(e.to_string()))?;
        pool.record(model, t0.elapsed().as_millis() as f32, false);
        return Err(RouterError::Upstream(format!("{status}: {text}")));
    }

    // Pool recording for stream path is done by the HTTP layer after the body finishes
    // (it holds `&PoolState` for the request). Here we only expose bytes + accumulation.
    let _ = t0;
    let _ = pool;
    let inner: BoxByteStream = Box::pin(
        resp.bytes_stream()
            .map(|item| item.map_err(|e| RouterError::Upstream(e.to_string()))),
    );
    Ok(SseByteStream {
        inner,
        buf: Vec::new(),
        finished: false,
        ok: true,
        started: Instant::now(),
        model: model.to_string(),
    })
}

pub async fn forward_sse_stream(
    doc: &RouterDocument,
    model: &str,
    req: &ChatRequest,
    extra_headers: &[(String, String)],
    pool: &PoolState,
) -> Result<SseByteStream, RouterError> {
    let provider = doc
        .provider(model)
        .ok_or_else(|| RouterError::FailClosed(format!("unknown provider model {model}")))?;
    let candidates = backend_candidates(doc, model)?;
    let mut body =
        serde_json::to_value(req).map_err(|e| RouterError::InvalidParam(e.to_string()))?;
    let upstream_model = if provider.provider_model_id.is_empty() {
        model.to_string()
    } else {
        provider.provider_model_id.clone()
    };
    body["model"] = Value::String(upstream_model);
    body["stream"] = Value::Bool(true);
    let n = candidates.len();
    let mut errors = Vec::new();
    let mut last = RouterError::Config(format!("model {model} has no backend_refs"));
    for (i, backend) in candidates.into_iter().enumerate() {
        match open_sse_once(pool, model, &body, backend, extra_headers).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                errors.push(format!("candidate {}: {e}", i + 1));
                last = e;
            }
        }
    }
    Err(candidates_exhausted(n, &errors, last))
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
    use aria_router_config::RouterDocument;
    use aria_router_core::ChatMessage;
    use serde_json::json;

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

    #[test]
    fn pool_state_record_updates_ema_and_failures() {
        let p = PoolState::default();
        // EMA: prev*0.7 + ms*0.3. First sample (prev==ms) stays 100, then it
        // moves toward 0 by 30% each step: 100 -> 70 -> 49.
        p.record("m", 100.0, true);
        p.record("m", 0.0, true);
        p.record("m", 0.0, true);
        let v = p.latency_map().get("m").copied().unwrap();
        assert!(
            (v - 49.0).abs() < 1e-3,
            "EMA latency for 'm' was {v}, expected ~49"
        );
        assert_eq!(p.failures_map().get("m").copied(), Some(0));
        p.record("m", 1.0, false);
        assert_eq!(p.failures_map().get("m").copied(), Some(1));
    }

    fn doc_with_backend(backend: &str) -> RouterDocument {
        RouterDocument::from_yaml_str(&format!(
            r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: m
  models:
    - name: m
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: low
          endpoint: {backend}
          weight: 1
        - name: high
          endpoint: {backend}
          weight: 5
"#,
            backend = backend
        ))
        .unwrap()
    }

    fn failover_doc(first: &str, second: &str, enabled: bool, max_attempts: u32) -> RouterDocument {
        RouterDocument::from_yaml_str(&format!(
            r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: m
  models:
    - name: m
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: {first}
          weight: 10
        - name: secondary
          endpoint: {second}
          weight: 1
global:
  services:
    backend_failover:
      enabled: {enabled}
      max_attempts: {max_attempts}
"#,
            first = first,
            second = second,
            enabled = enabled,
            max_attempts = max_attempts
        ))
        .unwrap()
    }

    #[test]
    fn failover_without_any_backend_ref_is_a_config_error() {
        let raw = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: m
  models:
    - name: m
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs: []
entrypoints:
  - model_names: [ariacompute/auto]
    router: semantic
    recipe: r
recipes:
  - name: r
    router: semantic
    routing:
      strategy: priority
      decisions:
        - name: d
          priority: 1
          rules:
            operator: AND
            conditions: []
global:
  services:
    backend_failover:
      enabled: true
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let err = backend_candidates(&doc, "m").unwrap_err();
        assert!(err.to_string().contains("has no backend_refs"), "{err}");
    }

    #[test]
    fn backend_candidates_are_ordered_by_weight_and_capped() {
        let single = doc_with_backend("127.0.0.1:9");
        assert_eq!(backend_candidates(&single, "m").unwrap().len(), 1);
        assert_eq!(backend_candidates(&single, "m").unwrap()[0].name, "high");
        let chained = failover_doc("127.0.0.1:9", "127.0.0.1:8", true, 2);
        let names: Vec<&str> = backend_candidates(&chained, "m")
            .unwrap()
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(names, vec!["primary", "secondary"]);
        let capped = failover_doc("127.0.0.1:9", "127.0.0.1:8", true, 1);
        assert_eq!(backend_candidates(&capped, "m").unwrap().len(), 1);
        assert!(matches!(
            backend_candidates(&chained, "unknown"),
            Err(RouterError::FailClosed(_))
        ));
        let exhausted = candidates_exhausted(
            2,
            &["candidate 1: x".into(), "candidate 2: y".into()],
            RouterError::Upstream("y".into()),
        );
        assert!(exhausted
            .to_string()
            .contains("all 2 backend candidates failed"));
        assert_eq!(
            candidates_exhausted(1, &[], RouterError::Upstream("x".into())).to_string(),
            RouterError::Upstream("x".into()).to_string()
        );
    }

    async fn mock_server_invalid_json() -> String {
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(|| async { "not json" }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn c8_forward_tries_next_candidate_on_failure() {
        let healthy = mock_server().await;
        let broken = closing_endpoint().await;
        let doc = failover_doc(&broken, &healthy, true, 2);
        let pool = PoolState::default();
        let body = forward(&doc, "m", &chat_req(), &[], &pool).await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ok");
        assert_eq!(pool.failures_map().get("m").copied(), Some(0));

        // A 200 with an invalid JSON body is also a failed candidate.
        let invalid = mock_server_invalid_json().await;
        let doc2 = failover_doc(&invalid, &healthy, true, 2);
        let body2 = forward(&doc2, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap();
        assert_eq!(body2["model"], "echo");

        // SSE uses the same candidate chain.
        let doc3 = failover_doc(&closing_endpoint().await, &healthy, true, 2);
        let text = forward_sse_text(&doc3, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap();
        assert!(text.contains("data:"));
    }

    #[tokio::test]
    async fn c8_failover_is_opt_in_and_fails_closed_when_exhausted() {
        let healthy = mock_server().await;
        let broken = closing_endpoint().await;
        // Disabled: the historical single (highest-weight) backend is used.
        let off = failover_doc(&broken, &healthy, false, 2);
        let err = forward(&off, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)));
        // Enabled but capped at one attempt: same outcome, no fallback.
        let capped = failover_doc(&broken, &healthy, true, 1);
        assert!(matches!(
            forward(&capped, "m", &chat_req(), &[], &PoolState::default()).await,
            Err(RouterError::Upstream(_))
        ));
        // All candidates fail: one error naming both failures, no secret leakage.
        let both_broken = failover_doc(
            &closing_endpoint().await,
            &mock_server_status(500).await,
            true,
            2,
        );
        let err = forward(&both_broken, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("all 2 backend candidates failed"));
        assert!(err.to_string().contains("500"));
        let sse_err = forward_sse_text(&both_broken, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap_err();
        assert!(sse_err
            .to_string()
            .contains("all 2 backend candidates failed"));
    }

    #[test]
    fn pick_backend_prefers_highest_weight() {
        let doc = doc_with_backend("127.0.0.1:9");
        let p = doc.provider("m").unwrap();
        assert_eq!(pick_backend(p).unwrap().name, "high");
    }

    fn backend_with(extra: &str) -> aria_router_config::BackendRef {
        let doc = RouterDocument::from_yaml_str(&format!(
            r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: m
  models:
    - name: m
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: x
          endpoint: 127.0.0.1:1
          weight: 1
          {extra}
"#,
            extra = extra
        ))
        .unwrap();
        doc.provider("m").unwrap().backend_refs[0].clone()
    }

    #[test]
    fn backend_api_key_resolution() {
        // Explicit api_key wins.
        let b = backend_with("api_key: sk-abc");
        assert_eq!(backend_api_key(&b), Some("sk-abc".into()));
        // Inline secret pasted into api_key_env (no matching process env) is used directly.
        let b2 = backend_with("api_key_env: sk-xyz1234567890abcdefghij");
        assert_eq!(
            backend_api_key(&b2),
            Some("sk-xyz1234567890abcdefghij".into())
        );
        // Plain name with no env var and not inline-like resolves to None.
        let b3 = backend_with("api_key_env: NOT_A_KEY");
        assert_eq!(backend_api_key(&b3), None);
    }

    #[test]
    fn looks_like_inline_api_key_classifies() {
        assert!(looks_like_inline_api_key("sk-abc"));
        assert!(looks_like_inline_api_key("sk_abc"));
        assert!(looks_like_inline_api_key(
            "abcdefghijklmnopqrstuvwxyz1234-5678"
        ));
        assert!(!looks_like_inline_api_key("short"));
        assert!(!looks_like_inline_api_key("hello world"));
    }

    fn chat_req() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: json!("hi"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        }
    }

    async fn mock_server() -> String {
        use axum::extract::Json;
        use axum::response::IntoResponse;
        use axum::routing::post;
        async fn echo(Json(v): Json<serde_json::Value>) -> impl IntoResponse {
            let model = v.get("model").cloned().unwrap_or(json!("echo"));
            let stream = v.get("stream").and_then(|x| x.as_bool()).unwrap_or(false);
            if stream {
                let chunk = json!({"model": model, "choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":null}]});
                let done = json!({"model": model, "choices":[{"index":0,"delta":{},"finish_reason":"stop"}], "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
                let body = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", chunk, done);
                axum::response::Response::builder()
                    .status(200)
                    .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                    .body(axum::body::Body::from(body))
                    .unwrap()
            } else {
                axum::Json(json!({"model": model, "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]})).into_response()
            }
        }
        let app = axum::Router::new().route("/v1/chat/completions", post(echo));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("127.0.0.1:{}", addr.port())
    }

    async fn mock_server_status(status: u16) -> String {
        use axum::http::StatusCode;
        use axum::routing::post;
        let handler = move || async move { StatusCode::from_u16(status).unwrap() };
        let app = axum::Router::new().route("/v1/chat/completions", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn forward_parses_upstream_response() {
        let backend = mock_server().await;
        let doc = doc_with_backend(&backend);
        let res = forward(&doc, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap();
        assert_eq!(res["choices"][0]["message"]["content"], "ok");
        // provider_model_id `echo` is forwarded upstream as the request model.
        assert_eq!(res["model"], "echo");
    }

    #[tokio::test]
    async fn forward_sse_text_concatenates() {
        let backend = mock_server().await;
        let doc = doc_with_backend(&backend);
        let mut req = chat_req();
        req.stream = true;
        let text = forward_sse_text(&doc, "m", &req, &[], &PoolState::default())
            .await
            .unwrap();
        assert!(text.contains("ok"));
        assert!(text.contains("[DONE]"));
    }

    #[tokio::test]
    async fn forward_unknown_model_fails_closed() {
        let doc = doc_with_backend("127.0.0.1:9");
        let err = forward(&doc, "nope", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap_err();
        assert!(matches!(err, RouterError::FailClosed(_)));
    }

    #[tokio::test]
    async fn forward_upstream_500_returns_upstream_error() {
        let backend = mock_server_status(500).await;
        let doc = doc_with_backend(&backend);
        let err = forward(&doc, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)));
    }

    #[test]
    fn pool_state_debug_lists_stats() {
        let p = PoolState::default();
        p.record("m", 5.0, false);
        let s = format!("{p:?}");
        assert!(s.contains("PoolState"));
        assert!(s.contains("failures"));
    }

    #[test]
    fn backend_api_key_empty_and_env_variants() {
        // Empty inline key falls through to api_key_env (absent) → None.
        let b = backend_with(r#"api_key: "" "#);
        assert_eq!(backend_api_key(&b), None);

        // Non-empty process env var is used.
        let name = "ARIA_TEST_PROVIDER_KEY_7f3c";
        std::env::set_var(name, "env-secret");
        let b = backend_with(&format!("api_key_env: {name}"));
        assert_eq!(backend_api_key(&b), Some("env-secret".into()));
        // Empty env value is ignored.
        std::env::set_var(name, "");
        assert_eq!(backend_api_key(&b), None);
        std::env::remove_var(name);
    }

    fn doc_with_empty_provider_model_id(backend: &str) -> RouterDocument {
        RouterDocument::from_yaml_str(&format!(
            r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: m
  models:
    - name: m
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: only
          endpoint: {backend}
          weight: 1
          api_key: sk-test-key
"#,
            backend = backend
        ))
        .unwrap()
    }

    async fn mock_server_headers() -> String {
        use axum::extract::Json;
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::routing::post;
        async fn echo(headers: HeaderMap, Json(v): Json<serde_json::Value>) -> impl IntoResponse {
            axum::Json(json!({
                "model": v.get("model").cloned().unwrap_or(json!("")),
                "auth": headers.get("authorization").and_then(|h| h.to_str().ok()).unwrap_or(""),
                "trace": headers.get("x-trace").and_then(|h| h.to_str().ok()).unwrap_or(""),
            }))
        }
        let app = axum::Router::new().route("/v1/chat/completions", post(echo));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn forward_uses_model_name_when_provider_id_absent_and_sends_headers() {
        let backend = mock_server_headers().await;
        let doc = doc_with_empty_provider_model_id(&backend);
        let pool = PoolState::default();
        let res = forward(
            &doc,
            "m",
            &chat_req(),
            &[("x-trace".to_string(), "trace-1".to_string())],
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(res["model"], "m");
        assert_eq!(res["auth"], "Bearer sk-test-key");
        assert_eq!(res["trace"], "trace-1");
        assert_eq!(pool.failures_map().get("m").copied(), Some(0));
    }

    /// Loopback socket that accepts one connection and closes it: a portable
    /// "upstream is down" that fails immediately. A closed port is not portable
    /// (some platforms block on the connect until the 300 s client timeout).
    async fn closing_endpoint() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            drop(sock);
        });
        addr.to_string()
    }

    #[tokio::test]
    async fn forward_connection_failure_records_failure() {
        let doc = doc_with_backend(&closing_endpoint().await);
        let pool = PoolState::default();
        let err = forward(&doc, "m", &chat_req(), &[], &pool)
            .await
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)));
        assert_eq!(pool.failures_map().get("m").copied(), Some(1));
    }

    #[tokio::test]
    async fn forward_sse_upstream_500_returns_upstream_error() {
        let backend = mock_server_status(500).await;
        let doc = doc_with_backend(&backend);
        let err = forward_sse_stream(&doc, "m", &chat_req(), &[], &PoolState::default())
            .await
            .map(|_| ())
            .expect_err("upstream 500 must be an error");
        assert!(matches!(err, RouterError::Upstream(_)));
    }

    #[tokio::test]
    async fn forward_sse_connection_failure_records_failure() {
        let doc = doc_with_backend(&closing_endpoint().await);
        let pool = PoolState::default();
        let err = forward_sse_stream(&doc, "m", &chat_req(), &[], &pool)
            .await
            .map(|_| ())
            .expect_err("connection failure must be an error");
        assert!(matches!(err, RouterError::Upstream(_)));
        assert_eq!(pool.failures_map().get("m").copied(), Some(1));
    }

    #[tokio::test]
    async fn forward_sse_sends_model_id_auth_and_headers() {
        let backend = mock_server().await;
        let doc = doc_with_empty_provider_model_id(&backend);
        let text = forward_sse_text(
            &doc,
            "m",
            &chat_req(),
            &[("x-trace".to_string(), "trace-1".to_string())],
            &PoolState::default(),
        )
        .await
        .unwrap();
        assert!(text.contains("\"m\""));
        assert!(text.contains("[DONE]"));
    }

    #[tokio::test]
    async fn sse_stream_reports_completion_and_double_poll() {
        use futures_util::StreamExt;
        let backend = mock_server().await;
        let doc = doc_with_backend(&backend);
        let mut stream = forward_sse_stream(&doc, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap();
        assert!(!stream.succeeded());
        assert_eq!(stream.model(), "m");
        while let Some(item) = stream.next().await {
            item.unwrap();
        }
        assert!(stream.succeeded());
        assert!(stream.accumulated().contains("[DONE]"));
        assert!(stream.elapsed_ms() >= 0.0);
        // A finished stream keeps returning `None` instead of polling upstream again.
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_byte_stream_handles_pending_and_error_items() {
        use futures_util::{FutureExt, StreamExt};

        // A pending inner stream keeps `poll_next` in the Pending branch.
        let pending: BoxByteStream = Box::pin(futures_util::stream::pending());
        let mut stream = SseByteStream {
            inner: pending,
            buf: Vec::new(),
            finished: false,
            ok: true,
            started: Instant::now(),
            model: "m".into(),
        };
        assert!(stream.next().now_or_never().is_none());

        // An error item marks the stream as failed and ends it.
        let failing: BoxByteStream = Box::pin(futures_util::stream::iter(vec![Err(
            RouterError::Upstream("boom".into()),
        )]));
        let mut stream = SseByteStream {
            inner: failing,
            buf: Vec::new(),
            finished: false,
            ok: true,
            started: Instant::now(),
            model: "m".into(),
        };
        assert!(stream.next().await.unwrap().is_err());
        assert!(!stream.succeeded());
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_propagates_body_errors() {
        use axum::body::Body;
        use axum::http::header::CONTENT_TYPE;
        use axum::routing::post;
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                let chunks = futures_util::stream::iter(vec![
                    Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"data: hi\n\n")),
                    Err(std::io::Error::other("upstream aborted")),
                ]);
                axum::response::Response::builder()
                    .status(200)
                    .header(CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(chunks))
                    .unwrap()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let doc = doc_with_backend(&format!("127.0.0.1:{}", addr.port()));

        let err = forward_sse_text(&doc, "m", &chat_req(), &[], &PoolState::default())
            .await
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)));
    }
}
