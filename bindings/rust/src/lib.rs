//! Native Rust SDK for aria-router (does not dlopen `libaria-router_ffi`).

use aria_router_config::RouterDocument;
use aria_router_core::RouterError;
use aria_router_http::{data_router, last_route_json, sdk_chat_complete, AppState};
use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

pub struct Router {
    state: Option<Arc<AppState>>,
    base_url: Option<String>,
    rt: tokio::runtime::Runtime,
    setup: SetupConfig,
    last_headers: Mutex<Value>,
}

/// Only `x-aria-router-*` decision headers are exposed; auth and upstream
/// headers must never leak into `last_route`.
const ROUTE_HEADERS: &[&str] = &[
    "x-aria-router-layer",
    "x-aria-router-decision",
    "x-aria-router-model",
    "x-aria-router-algorithm",
    "x-aria-router-reason",
    "x-aria-router-confidence",
    "x-aria-router-bypass",
    "x-aria-router-latency-ms",
    "x-aria-router-redecision",
    "x-aria-router-retention-drop",
    "x-aria-router-retention-ttl-turns",
    "x-aria-router-retention-keep-current-model",
    "x-aria-router-retention-prefer-prefix",
    "x-aria-router-semantic-cache",
];

fn route_headers(headers: &HeaderMap) -> Value {
    let mut out = serde_json::Map::new();
    for name in ROUTE_HEADERS {
        if let Some(value) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            out.insert((*name).to_string(), serde_json::json!(value));
        }
    }
    Value::Object(out)
}

#[derive(Debug, Clone, Default)]
pub struct SetupConfig {
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Default)]
pub struct SetupUpdates {
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
            setup: SetupConfig::default(),
            last_headers: Mutex::new(serde_json::json!({})),
        }
    }

    fn clear_last_headers(&self) {
        *self.last_headers.lock().unwrap() = serde_json::json!({});
    }

    fn set_last_headers(&self, headers: Value) {
        *self.last_headers.lock().unwrap() = headers;
    }

    pub fn setup(&mut self, u: &SetupUpdates) -> Result<&mut Self, RouterError> {
        if let Some(v) = &u.base_url {
            self.setup.base_url = v.clone();
        }
        if let Some(v) = &u.token {
            self.setup.token = v.clone();
        }
        Ok(self)
    }

    pub fn setup_status(&self) -> &SetupConfig {
        &self.setup
    }

    pub fn setup_clear(&mut self) -> &mut Self {
        self.setup = SetupConfig::default();
        self
    }

    /// Load YAML. Empty `config_path` → `~/.ariacompute/router.yml` (`default_config_path`).
    pub fn init(&mut self, config_path: &str) -> Result<&mut Self, RouterError> {
        let path = if config_path.is_empty() {
            aria_router_config::default_config_path()?
        } else {
            let t = config_path.trim();
            if t.starts_with("~/") {
                aria_router_config::resolve_home_path(t, aria_router_config::default_config_path)?
            } else {
                std::path::PathBuf::from(t)
            }
        };
        let doc = RouterDocument::load_path(&path)?;
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
        self.clear_last_headers();
        let model = options
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("ariacompute/semantic-auto");
        let mut req = serde_json::json!({"model": model, "messages": messages});
        if let Some(mt) = options.get("max_tokens") {
            req["max_tokens"] = mt.clone();
        }
        if let Some(st) = &self.state {
            let resp = if !self.setup.token.is_empty() {
                let app = data_router(st.clone());
                let body = serde_json::to_vec(&req).unwrap();
                self.rt
                    .block_on(async {
                        app.oneshot(
                            Request::post("/v1/chat/completions")
                                .header("content-type", "application/json")
                                .header("authorization", format!("Bearer {}", self.setup.token))
                                .body(Body::from(body))
                                .unwrap(),
                        )
                        .await
                    })
                    .map_err(|e| RouterError::Upstream(e.to_string()))?
            } else {
                self.rt
                    .block_on(sdk_chat_complete(st.clone(), req))
                    .map_err(|e| RouterError::Upstream(e.to_string()))?
            };
            let headers = route_headers(resp.headers());
            let status = resp.status();
            let bytes = self
                .rt
                .block_on(to_bytes(resp.into_body(), 1 << 22))
                .map_err(|e| RouterError::Upstream(e.to_string()))?;
            if status != StatusCode::OK {
                return Err(RouterError::Upstream(
                    String::from_utf8_lossy(&bytes).into_owned(),
                ));
            }
            self.set_last_headers(headers);
            serde_json::from_slice(&bytes).map_err(|e| RouterError::Upstream(e.to_string()))
        } else {
            let url = self
                .base_url
                .as_deref()
                .or(if self.setup.base_url.is_empty() {
                    None
                } else {
                    Some(self.setup.base_url.as_str())
                })
                .ok_or_else(|| RouterError::InvalidParam("not initialized".into()))?;
            let (text, headers) = self
                .rt
                .block_on(async {
                    let client = reqwest::Client::new();
                    let mut builder = client
                        .post(format!("{}/v1/chat/completions", url.trim_end_matches('/')))
                        .json(&req);
                    if !self.setup.token.is_empty() {
                        builder =
                            builder.header("authorization", format!("Bearer {}", self.setup.token));
                    }
                    let resp = builder.send().await.map_err(|e| e.to_string())?;
                    let headers = route_headers(resp.headers());
                    let status = resp.status();
                    let text = resp.text().await.map_err(|e| e.to_string())?;
                    if !status.is_success() {
                        return Err(format!("remote status {status}: {text}"));
                    }
                    Ok::<(String, Value), String>((text, headers))
                })
                .map_err(RouterError::Upstream)?;
            self.set_last_headers(headers);
            serde_json::from_str(&text).map_err(|e| RouterError::Upstream(e.to_string()))
        }
    }

    pub fn models(&self) -> Result<Value, RouterError> {
        if let Some(st) = &self.state {
            let doc = st.doc.lock().unwrap();
            let data: Vec<_> = doc
                .entrypoints
                .iter()
                .flat_map(|e| e.model_names.iter().cloned())
                .collect();
            return Ok(serde_json::json!({"data": data}));
        }
        let url = self
            .base_url
            .as_deref()
            .or(if self.setup.base_url.is_empty() {
                None
            } else {
                Some(self.setup.base_url.as_str())
            })
            .ok_or_else(|| RouterError::InvalidParam("not initialized".into()))?;
        let text = self
            .rt
            .block_on(async {
                let client = reqwest::Client::new();
                let mut builder = client.get(format!("{}/v1/models", url.trim_end_matches('/')));
                if !self.setup.token.is_empty() {
                    builder =
                        builder.header("authorization", format!("Bearer {}", self.setup.token));
                }
                let resp = builder.send().await.map_err(|e| e.to_string())?;
                let status = resp.status();
                let text = resp.text().await.map_err(|e| e.to_string())?;
                if !status.is_success() {
                    return Err(format!("remote status {status}: {text}"));
                }
                Ok::<String, String>(text)
            })
            .map_err(RouterError::Upstream)?;
        serde_json::from_str(&text).map_err(|e| RouterError::Upstream(e.to_string()))
    }

    /// The native SDK mirrors the C ABI's single full-body chunk callback.
    pub fn complete_stream(
        &self,
        messages: Value,
        options: Value,
        on_chunk: Option<&dyn Fn(&str)>,
    ) -> Result<Value, RouterError> {
        let out = self.complete(messages, options)?;
        if let Some(cb) = on_chunk {
            cb(&out.to_string());
        }
        Ok(out)
    }

    pub fn last_route(&self) -> Value {
        let mut v = self
            .state
            .as_ref()
            .map(|s| last_route_json(s))
            .unwrap_or(serde_json::json!({}));
        let headers = self.last_headers.lock().unwrap().clone();
        if headers.as_object().is_some_and(|o| !o.is_empty()) {
            v["headers"] = headers;
        }
        v
    }

    /// Reset the handle; the native SDK owns no FFI handle to free.
    pub fn close(&mut self) {
        self.state = None;
        self.base_url = None;
        self.clear_last_headers();
    }

    pub fn destroy(&mut self) {
        self.close();
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
        let path = if p1.exists() { p1 } else { p2 };
        r.init(path.to_str().unwrap()).unwrap();
        let m = r.models().unwrap();
        assert!(m["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "ariacompute/semantic-auto"));
    }

    #[test]
    fn setup_memory_only() {
        let mut r = Router::new();
        r.setup(&SetupUpdates {
            base_url: Some("http://127.0.0.1:8899".into()),
            token: Some("t".into()),
        })
        .unwrap();
        assert_eq!(r.setup_status().token, "t");
        r.setup_clear();
        assert!(r.setup_status().token.is_empty());
    }

    #[test]
    fn complete_require_api_key_without_token() {
        let mut r = Router::new();
        let dir = std::env::temp_dir().join(format!(
            "aria-router-sdk-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("fast-response-require-key.yaml");
        // Test cwd differs between `cargo test -p` and workspace runs.
        let candidates = [
            std::path::Path::new("../../bindings/testdata/fast-response.yaml"),
            std::path::Path::new("bindings/testdata/fast-response.yaml"),
            std::path::Path::new("testdata/fast-response.yaml"),
        ];
        let path = candidates
            .iter()
            .find(|p| p.exists())
            .expect("fast-response fixture");
        let fixture = std::fs::read_to_string(path).unwrap();
        let yaml = fixture.replace("require_api_key: false", "require_api_key: true");
        std::fs::write(&cfg, &yaml).unwrap();
        r.init(cfg.to_str().unwrap()).unwrap();
        let out = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({"model":"ariacompute/semantic-auto"}),
            )
            .unwrap();
        let s = out.to_string();
        assert!(s.contains("hello-from-router"), "{s}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fixture_path() -> &'static std::path::Path {
        let candidates = [
            std::path::Path::new("../../bindings/testdata/fast-response.yaml"),
            std::path::Path::new("bindings/testdata/fast-response.yaml"),
            std::path::Path::new("testdata/fast-response.yaml"),
        ];
        candidates
            .iter()
            .find(|p| p.exists())
            .copied()
            .expect("fast-response fixture")
    }

    fn write_fixture(dir: &std::path::Path, name: &str, require_key: bool) -> std::path::PathBuf {
        let mut yaml = std::fs::read_to_string(fixture_path()).unwrap();
        if require_key {
            yaml = yaml.replace("require_api_key: false", "require_api_key: true");
        }
        let path = dir.join(name);
        std::fs::write(&path, yaml).unwrap();
        path
    }

    fn restore_home(prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var("ARIA_COMPUTE_HOME", v),
            None => std::env::remove_var("ARIA_COMPUTE_HOME"),
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aria-router-sdk-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_router_is_uninitialized() {
        let r = Router::default();
        assert!(matches!(
            r.models().unwrap_err(),
            RouterError::InvalidParam(_)
        ));
        assert_eq!(r.last_route(), serde_json::json!({}));
        let err = r
            .complete(serde_json::json!([]), serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, RouterError::InvalidParam(_)), "{err}");
    }

    #[test]
    fn init_empty_and_tilde_paths() {
        let dir = temp_dir("home");
        std::fs::write(
            dir.join("router.yml"),
            std::fs::read(fixture_path()).unwrap(),
        )
        .unwrap();
        let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
        std::env::set_var("ARIA_COMPUTE_HOME", &dir);
        let mut r = Router::new();
        r.init("").unwrap();
        assert_eq!(
            r.setup_status().base_url,
            "",
            "init must not touch setup state"
        );
        // Both restore arms (var was present / absent) must behave.
        restore_home(Some(dir.to_string_lossy().into_owned()));
        restore_home(prev);

        // `~/` expansion goes through resolve_home_path.
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .expect("home dir");
        let sub = home
            .join(".ariacompute")
            .join("tmp")
            .join(format!("sdk-tilde-{}", std::process::id()));
        std::fs::create_dir_all(&sub).unwrap();
        let cfg = sub.join("tilde.yaml");
        std::fs::write(&cfg, std::fs::read(fixture_path()).unwrap()).unwrap();
        let rel = cfg
            .strip_prefix(&home)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut r = Router::new();
        r.init(&format!("~/{rel}")).unwrap();
        let _ = std::fs::remove_dir_all(&sub);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn connect_mode_models_route_and_complete() {
        // Accept then close: a portable "upstream down" that fails at once.
        // A closed port is not portable — some platforms block on the connect
        // instead of refusing, and the SDK client sets no timeout.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || loop {
            let (sock, _) = listener.accept().unwrap();
            drop(sock);
        });

        let mut r = Router::new();
        r.connect(&endpoint);
        let err = r.models().unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)), "{err}");
        assert_eq!(r.last_route(), serde_json::json!({}));
        // Connection refused → Upstream, not a silent empty result.
        let err = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({}),
            )
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)), "{err}");
    }

    #[test]
    fn in_process_complete_with_and_without_token() {
        let dir = temp_dir("inproc");
        let cfg = write_fixture(&dir, "fast.yaml", false);
        let mut r = Router::new();
        r.init(cfg.to_str().unwrap()).unwrap();
        let out = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({"model":"ariacompute/semantic-auto","max_tokens":11}),
            )
            .unwrap();
        assert!(out.to_string().contains("hello-from-router"));

        // An explicit token goes through the authorized data-plane path; an
        // unknown key surfaces the upstream 401 body instead of a silent success.
        r.setup(&SetupUpdates {
            base_url: None,
            token: Some("sk-unknown".into()),
        })
        .unwrap();
        let err = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({}),
            )
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)), "{err}");
        assert!(err.to_string().contains("unauthorized"), "{err}");

        // Key-requiring config without a token stays on the SDK bypass path.
        let cfg2 = write_fixture(&dir, "require.yaml", true);
        r.init(cfg2.to_str().unwrap()).unwrap();
        r.setup_clear();
        let out = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({}),
            )
            .unwrap();
        assert!(out.to_string().contains("hello-from-router"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct Stub {
        endpoint: String,
        rt: tokio::runtime::Runtime,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl Stub {
        fn start() -> Self {
            let app = axum::Router::new()
                .route(
                    "/v1/chat/completions",
                    axum::routing::post(
                        |headers: axum::http::HeaderMap,
                         axum::Json(body): axum::Json<Value>| async move {
                            let mut resp_headers = HeaderMap::new();
                            resp_headers.insert(
                                "x-aria-router-layer",
                                axum::http::HeaderValue::from_static("semantic"),
                            );
                            resp_headers.insert(
                                "x-aria-router-redecision",
                                axum::http::HeaderValue::from_static("new-user-message"),
                            );
                            resp_headers.insert(
                                "authorization",
                                axum::http::HeaderValue::from_static("must-not-leak"),
                            );
                            (
                                resp_headers,
                                axum::Json(serde_json::json!({
                                    "auth": headers
                                        .get("authorization")
                                        .and_then(|h| h.to_str().ok())
                                        .unwrap_or(""),
                                    "model": body.get("model").cloned().unwrap_or(Value::Null),
                                })),
                            )
                        },
                    ),
                )
                .route(
                    "/v1/models",
                    axum::routing::get(|| async {
                        axum::Json(serde_json::json!({
                            "object": "list",
                            "data": [{"id": "stub/model", "object": "model", "owned_by": "stub"}]
                        }))
                    }),
                )
                .route(
                    "/fail/v1/models",
                    axum::routing::get(|| async {
                        (StatusCode::INTERNAL_SERVER_ERROR, "models boom")
                    }),
                )
                .route(
                    "/fail/v1/chat/completions",
                    axum::routing::post(|| async {
                        (StatusCode::INTERNAL_SERVER_ERROR, "chat boom")
                    }),
                );
            let rt = tokio::runtime::Runtime::new().unwrap();
            let (addr, stop, handle) = rt.block_on(async {
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
                (addr, tx, handle)
            });
            Self {
                endpoint: format!("http://{addr}"),
                rt,
                stop: Some(stop),
                handle: Some(handle),
            }
        }

        fn shutdown(mut self) {
            let tx = self.stop.take().expect("stop");
            let handle = self.handle.take().expect("handle");
            let _ = tx.send(());
            let _ = self.rt.block_on(handle);
        }
    }

    #[test]
    fn route_headers_whitelist_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-aria-router-layer",
            axum::http::HeaderValue::from_static("semantic"),
        );
        headers.insert(
            "authorization",
            axum::http::HeaderValue::from_static("secret"),
        );
        headers.insert(
            "x-aria-router-reason",
            axum::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
        );
        let v = route_headers(&headers);
        assert_eq!(v["x-aria-router-layer"], "semantic");
        assert!(v.get("authorization").is_none());
        assert!(v.get("x-aria-router-reason").is_none());
        assert_eq!(route_headers(&HeaderMap::new()), serde_json::json!({}));
    }

    #[test]
    fn remote_models_headers_and_failures() {
        let stub = Stub::start();
        let mut r = Router::new();
        r.connect(&stub.endpoint);
        r.setup(&SetupUpdates {
            base_url: None,
            token: Some("sk-remote".into()),
        })
        .unwrap();
        let models = r.models().unwrap();
        assert_eq!(models["data"][0]["id"], "stub/model");
        let out = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(out["auth"], "Bearer sk-remote");
        let lr = r.last_route();
        assert_eq!(lr["headers"]["x-aria-router-layer"], "semantic");
        assert_eq!(
            lr["headers"]["x-aria-router-redecision"],
            "new-user-message"
        );
        assert!(lr["headers"].get("authorization").is_none());

        let mut failing = Router::new();
        failing.connect(&format!("{}/fail", stub.endpoint));
        let err = failing.models().unwrap_err();
        assert!(err.to_string().contains("remote status 500"), "{err}");
        let err = failing
            .complete(serde_json::json!([]), serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("remote status 500"), "{err}");
        assert_eq!(failing.last_route(), serde_json::json!({}));
        stub.shutdown();
    }

    #[test]
    fn complete_stream_and_close() {
        let dir = temp_dir("stream");
        let cfg = write_fixture(&dir, "fast.yaml", false);
        let mut r = Router::new();
        r.init(cfg.to_str().unwrap()).unwrap();
        let chunks = Mutex::new(Vec::<String>::new());
        let cb: &dyn Fn(&str) = &|chunk: &str| chunks.lock().unwrap().push(chunk.to_string());
        let out = r
            .complete_stream(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({}),
                Some(cb),
            )
            .unwrap();
        assert_eq!(chunks.lock().unwrap().len(), 1);
        assert_eq!(chunks.lock().unwrap()[0], out.to_string());
        assert!(out.to_string().contains("hello-from-router"));
        assert!(r.last_route()["headers"]["x-aria-router-layer"].is_string());
        r.complete_stream(serde_json::json!([]), serde_json::json!({}), None)
            .unwrap();
        r.close();
        assert!(r.models().is_err());
        assert_eq!(r.last_route(), serde_json::json!({}));
        r.destroy();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_complete_and_base_url_fallbacks() {
        let stub = Stub::start();
        let mut r = Router::new();
        // `connect` takes precedence over `setup.base_url`.
        r.connect(&stub.endpoint);
        r.setup(&SetupUpdates {
            base_url: Some("http://127.0.0.1:1".into()),
            token: Some("sk-remote".into()),
        })
        .unwrap();
        let out = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({"model":"remote/model"}),
            )
            .unwrap();
        assert_eq!(out["auth"], "Bearer sk-remote");
        assert_eq!(out["model"], "remote/model");

        // Without `connect`, setup.base_url is used.
        let mut r = Router::new();
        r.setup(&SetupUpdates {
            base_url: Some(stub.endpoint.clone()),
            token: None,
        })
        .unwrap();
        let out = r
            .complete(
                serde_json::json!([{"role":"user","content":"hi"}]),
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(out["auth"], "");
        assert_eq!(out["model"], "ariacompute/semantic-auto");
        // Without connect(), setup.base_url also drives models() over HTTP.
        let models = r.models().unwrap();
        assert_eq!(models["data"][0]["id"], "stub/model");

        // Empty setup.base_url + no connect → "not initialized".
        let mut r = Router::new();
        r.setup(&SetupUpdates {
            base_url: Some(String::new()),
            token: None,
        })
        .unwrap();
        let err = r
            .complete(serde_json::json!([]), serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, RouterError::InvalidParam(_)), "{err}");
        stub.shutdown();
    }
}
