//! C ABI for aria-router (`libaria-router_ffi`).

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI: pointers are caller-owned

use aria_router_config::RouterDocument;
use aria_router_http::{data_router, last_route_json, sdk_chat_complete, AppState};
use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

thread_local! {
    static LAST_ERROR: Mutex<Option<CString>> = const { Mutex::new(None) };
}

fn set_err(msg: &str) {
    LAST_ERROR.with(|s| {
        *s.lock().unwrap() =
            Some(CString::new(msg).unwrap_or_else(|_| CString::new("err").unwrap()));
    });
}

#[no_mangle]
pub extern "C" fn aria_router_last_error() -> *const c_char {
    LAST_ERROR.with(|s| {
        s.lock()
            .unwrap()
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

pub struct AriaRouterHandle {
    state: Option<Arc<AppState>>,
    base_url: Option<String>,
    token: Option<String>,
    last_headers: Mutex<serde_json::Value>,
    rt: tokio::runtime::Runtime,
}

impl AriaRouterHandle {
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("rt")
    }

    fn clear_last_headers(&self) {
        *self.last_headers.lock().unwrap() = serde_json::json!({});
    }

    fn set_last_headers(&self, headers: serde_json::Value) {
        *self.last_headers.lock().unwrap() = headers;
    }
}

fn cstr<'a>(p: *const c_char) -> Result<&'a str, ()> {
    if p.is_null() {
        return Err(());
    }
    unsafe { CStr::from_ptr(p).to_str().map_err(|_| ()) }
}

fn opt_cstr_update(dest: &mut Option<String>, p: *const c_char) {
    if p.is_null() {
        return;
    }
    match cstr(p) {
        Ok("") => *dest = None,
        Ok(s) => *dest = Some(s.to_string()),
        Err(()) => {}
    }
}

#[no_mangle]
pub extern "C" fn aria_router_init(config_path: *const c_char) -> *mut AriaRouterHandle {
    let path = match resolve_init_config_path(config_path) {
        Ok(p) => p,
        Err(e) => {
            set_err(&e);
            return std::ptr::null_mut();
        }
    };
    match RouterDocument::load_path(&path) {
        Ok(doc) => {
            let h = Box::new(AriaRouterHandle {
                state: Some(Arc::new(AppState::new(doc))),
                base_url: None,
                token: None,
                last_headers: Mutex::new(serde_json::json!({})),
                rt: AriaRouterHandle::runtime(),
            });
            Box::into_raw(h)
        }
        Err(e) => {
            set_err(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

fn resolve_init_config_path(config_path: *const c_char) -> Result<std::path::PathBuf, String> {
    if !config_path.is_null() {
        match cstr(config_path) {
            Ok(s) if !s.is_empty() => {
                let t = s.trim();
                // Expand `~/…` (shell does not expand inside library calls).
                if t.starts_with("~/") {
                    return aria_router_config::resolve_home_path(
                        t,
                        aria_router_config::default_config_path,
                    )
                    .map_err(|e| e.to_string());
                }
                return Ok(std::path::PathBuf::from(t));
            }
            Ok(_) => {}
            Err(()) => return Err("invalid config_path".into()),
        }
    }
    aria_router_config::default_config_path().map_err(|e| e.to_string())
}

#[no_mangle]
pub extern "C" fn aria_router_connect(base_url: *const c_char) -> *mut AriaRouterHandle {
    let url = match cstr(base_url) {
        Ok(s) => s.to_string(),
        Err(()) => {
            set_err("null base_url");
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(AriaRouterHandle {
        state: None,
        base_url: Some(url),
        token: None,
        last_headers: Mutex::new(serde_json::json!({})),
        rt: AriaRouterHandle::runtime(),
    }))
}

/// In-memory client auth only (never writes router.yml).
/// Non-NULL `base_url` / `token` update that field; empty string clears; NULL leaves unchanged.
#[no_mangle]
pub extern "C" fn aria_router_setup(
    router: *mut AriaRouterHandle,
    base_url: *const c_char,
    token: *const c_char,
) {
    if router.is_null() {
        return;
    }
    let h = unsafe { &mut *router };
    opt_cstr_update(&mut h.base_url, base_url);
    opt_cstr_update(&mut h.token, token);
}

#[no_mangle]
pub extern "C" fn aria_router_destroy(router: *mut AriaRouterHandle) {
    if !router.is_null() {
        unsafe {
            drop(Box::from_raw(router));
        }
    }
}

fn write_out(out: *mut c_char, out_len: usize, s: &str) -> i32 {
    if out.is_null() || out_len == 0 {
        set_err("null out");
        return -1;
    }
    let bytes = s.as_bytes();
    if bytes.len() + 1 > out_len {
        set_err("out buffer too small");
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
        *out.add(bytes.len()) = 0;
    }
    0
}

fn token_nonempty(h: &AriaRouterHandle) -> Option<&str> {
    h.token.as_deref().filter(|t| !t.is_empty())
}

/// Only `x-aria-router-*` decision headers are exposed to SDK consumers; auth,
/// cookies and arbitrary upstream headers must never leak into `last_route`.
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

fn route_headers(headers: &HeaderMap) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for name in ROUTE_HEADERS {
        if let Some(value) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            out.insert((*name).to_string(), serde_json::json!(value));
        }
    }
    serde_json::Value::Object(out)
}

fn remote_get(h: &AriaRouterHandle, url: &str) -> Result<String, String> {
    h.rt.block_on(async {
        let client = reqwest::Client::new();
        let mut req = client.get(url);
        if let Some(tok) = token_nonempty(h) {
            req = req.header("authorization", format!("Bearer {tok}"));
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("remote status {status}: {text}"));
        }
        Ok(text)
    })
}

fn complete_inner(
    h: &AriaRouterHandle,
    messages_json: &str,
    options_json: &str,
) -> Result<String, String> {
    h.clear_last_headers();
    let opts: serde_json::Value = serde_json::from_str(if options_json.is_empty() {
        "{}"
    } else {
        options_json
    })
    .map_err(|e| e.to_string())?;
    let model = opts
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("ariacompute/semantic-auto");
    let messages: serde_json::Value =
        serde_json::from_str(messages_json).map_err(|e| e.to_string())?;
    let mut req_json = serde_json::json!({
        "model": model,
        "messages": messages,
    });
    if let Some(mt) = opts.get("max_tokens") {
        req_json["max_tokens"] = mt.clone();
    }
    if let Some(st) = &h.state {
        let resp = if let Some(tok) = token_nonempty(h) {
            let app = data_router(st.clone());
            let body = serde_json::to_vec(&req_json).unwrap();
            h.rt.block_on(async {
                app.oneshot(
                    Request::post("/v1/chat/completions")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {tok}"))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
            })
            .map_err(|e| e.to_string())?
        } else {
            h.rt.block_on(sdk_chat_complete(st.clone(), req_json))
                .map_err(|e| e.to_string())?
        };
        let headers = route_headers(resp.headers());
        let status = resp.status();
        let bytes =
            h.rt.block_on(to_bytes(resp.into_body(), 1 << 22))
                .map_err(|e| e.to_string())?;
        if status != StatusCode::OK {
            return Err(String::from_utf8_lossy(&bytes).into_owned());
        }
        h.set_last_headers(headers);
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    } else if let Some(url) = &h.base_url {
        let url = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
        let resp =
            h.rt.block_on(async {
                let client = reqwest::Client::new();
                let mut req = client.post(&url).json(&req_json);
                if let Some(tok) = token_nonempty(h) {
                    req = req.header("authorization", format!("Bearer {tok}"));
                }
                req.send().await
            })
            .map_err(|e| e.to_string())?;
        let headers = route_headers(resp.headers());
        let status = resp.status();
        let text = h.rt.block_on(resp.text()).map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("remote status {status}: {text}"));
        }
        h.set_last_headers(headers);
        Ok(text)
    } else {
        Err("uninitialized".into())
    }
}

#[no_mangle]
pub extern "C" fn aria_router_complete(
    router: *mut AriaRouterHandle,
    messages_json: *const c_char,
    options_json: *const c_char,
    out: *mut c_char,
    out_len: usize,
) -> i32 {
    if router.is_null() {
        set_err("null router");
        return -1;
    }
    let h = unsafe { &*router };
    let messages = cstr(messages_json).unwrap_or("[]");
    let options = cstr(options_json).unwrap_or("{}");
    match complete_inner(h, messages, options) {
        Ok(s) => write_out(out, out_len, &s),
        Err(e) => {
            set_err(&e);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn aria_router_complete_stream(
    router: *mut AriaRouterHandle,
    messages_json: *const c_char,
    options_json: *const c_char,
    out: *mut c_char,
    out_len: usize,
    callback: Option<extern "C" fn(*const c_char, *mut libc::c_void)>,
    user_data: *mut libc::c_void,
) -> i32 {
    let rc = aria_router_complete(router, messages_json, options_json, out, out_len);
    if rc == 0 {
        if let Some(cb) = callback {
            cb(out, user_data);
        }
    }
    rc
}

#[no_mangle]
pub extern "C" fn aria_router_models(
    router: *mut AriaRouterHandle,
    out: *mut c_char,
    out_len: usize,
) -> i32 {
    if router.is_null() {
        set_err("null router");
        return -1;
    }
    if out.is_null() || out_len == 0 {
        return write_out(out, out_len, "");
    }
    let h = unsafe { &*router };
    if let Some(st) = &h.state {
        let doc = st.doc.lock().unwrap();
        let ids: Vec<String> = doc
            .entrypoints
            .iter()
            .flat_map(|e| e.model_names.clone())
            .chain(doc.providers.models.iter().map(|m| m.name.clone()))
            .collect();
        return write_out(out, out_len, &serde_json::json!({"data": ids}).to_string());
    }
    if let Some(url) = &h.base_url {
        let url = format!("{}/v1/models", url.trim_end_matches('/'));
        return match remote_get(h, &url) {
            Ok(text) => write_out(out, out_len, &text),
            Err(e) => {
                set_err(&e);
                -1
            }
        };
    }
    set_err("uninitialized");
    -1
}

#[no_mangle]
pub extern "C" fn aria_router_last_route(
    router: *mut AriaRouterHandle,
    out: *mut c_char,
    out_len: usize,
) -> i32 {
    if router.is_null() {
        set_err("null router");
        return -1;
    }
    let h = unsafe { &*router };
    let mut v = if let Some(st) = &h.state {
        last_route_json(st)
    } else {
        serde_json::json!({})
    };
    let headers = h.last_headers.lock().unwrap().clone();
    if headers.as_object().is_some_and(|o| !o.is_empty()) {
        v["headers"] = headers;
    }
    write_out(out, out_len, &v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::ffi::{CStr, CString};

    /// Read the thread-local last error eagerly (the `assert!` format arguments
    /// are lazy, which would leave the error path unexercised on success).
    fn last_error() -> String {
        let p = aria_router_last_error();
        if p.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    fn restore_home(prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var("ARIA_COMPUTE_HOME", v),
            None => std::env::remove_var("ARIA_COMPUTE_HOME"),
        }
    }

    #[test]
    fn init_missing() {
        let p = CString::new("/no/such/config.yaml").unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(h.is_null());
        assert!(!aria_router_last_error().is_null());
    }

    #[test]
    fn init_defaults_and_invalid_paths() {
        // NULL config path → `default_config_path()` under ARIA_COMPUTE_HOME.
        let dir = tempfile_dir();
        std::fs::write(
            dir.join("router.yml"),
            include_str!("../../config/examples/semantic-tiny.yaml"),
        )
        .unwrap();
        let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
        std::env::set_var("ARIA_COMPUTE_HOME", &dir);
        let h = aria_router_init(std::ptr::null());
        assert!(!h.is_null());
        aria_router_destroy(h);
        // Empty string is treated like NULL.
        let empty = CString::new("").unwrap();
        let h = aria_router_init(empty.as_ptr());
        assert!(!h.is_null());
        aria_router_destroy(h);
        // Exercise both restore arms (var was present / absent).
        restore_home(Some(dir.to_string_lossy().into_owned()));
        restore_home(prev);
        let _ = std::fs::remove_dir_all(&dir);

        // Non-UTF-8 path is rejected with a clear error.
        let bad = unsafe { CString::from_vec_unchecked(vec![0xff, 0xfe, 0x00]) };
        let h = aria_router_init(bad.as_ptr());
        assert!(h.is_null());
        let msg = unsafe {
            CStr::from_ptr(aria_router_last_error())
                .to_string_lossy()
                .into_owned()
        };
        assert!(msg.contains("invalid config_path"), "{msg}");
    }

    #[test]
    fn connect_null_and_success() {
        let h = aria_router_connect(std::ptr::null());
        assert!(h.is_null());
        let msg = unsafe {
            CStr::from_ptr(aria_router_last_error())
                .to_string_lossy()
                .into_owned()
        };
        assert!(msg.contains("null base_url"), "{msg}");

        let url = CString::new("http://127.0.0.1:1").unwrap();
        let h = aria_router_connect(url.as_ptr());
        assert!(!h.is_null());
        assert!(unsafe { &*h }.state.is_none());
        assert_eq!(
            unsafe { &*h }.base_url.as_deref(),
            Some("http://127.0.0.1:1")
        );
        aria_router_destroy(h);
    }

    #[test]
    fn null_handle_guards() {
        aria_router_setup(std::ptr::null_mut(), std::ptr::null(), std::ptr::null());
        let mut buf = vec![0u8; 64];
        let msgs = CString::new("[]").unwrap();
        assert_eq!(
            aria_router_complete(
                std::ptr::null_mut(),
                msgs.as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            ),
            -1
        );
        assert_eq!(
            aria_router_models(
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len()
            ),
            -1
        );
        assert_eq!(
            aria_router_last_route(
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len()
            ),
            -1
        );
        let msg = unsafe {
            CStr::from_ptr(aria_router_last_error())
                .to_string_lossy()
                .into_owned()
        };
        assert!(msg.contains("null router"), "{msg}");
    }

    #[test]
    fn write_out_guards_and_connected_handle_views() {
        let stub = StubChat::start();
        let url = CString::new(stub.endpoint.clone()).unwrap();
        let h = aria_router_connect(url.as_ptr());
        assert!(!h.is_null());
        let mut buf = vec![0u8; 4096];

        // NULL / zero-length out buffers are rejected before any I/O.
        assert_eq!(aria_router_models(h, std::ptr::null_mut(), 0), -1);
        let msg = unsafe {
            CStr::from_ptr(aria_router_last_error())
                .to_string_lossy()
                .into_owned()
        };
        assert!(msg.contains("null out"), "{msg}");
        // Too-small buffer is rejected after a successful HTTP call.
        let mut tiny = vec![0u8; 2];
        assert_eq!(
            aria_router_models(h, tiny.as_mut_ptr() as *mut c_char, tiny.len()),
            -1
        );
        let msg = unsafe {
            CStr::from_ptr(aria_router_last_error())
                .to_string_lossy()
                .into_owned()
        };
        assert!(msg.contains("out buffer too small"), "{msg}");

        // Connected handles proxy /v1/models; last_route is empty before a chat.
        assert_eq!(
            aria_router_models(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            0
        );
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_string_lossy();
        assert!(s.contains("stub/model"), "{s}");
        assert_eq!(
            aria_router_last_route(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            0
        );
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_string_lossy();
        assert_eq!(s.as_ref(), "{}");

        // Clearing base_url leaves the handle uninitialized → complete/models fail.
        let clear = CString::new("").unwrap();
        aria_router_setup(h, clear.as_ptr(), std::ptr::null());
        let msgs = CString::new("[]").unwrap();
        assert_eq!(
            aria_router_complete(
                h,
                msgs.as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            ),
            -1
        );
        assert!(last_error().contains("uninitialized"), "{}", last_error());
        assert_eq!(
            aria_router_models(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            -1
        );
        assert!(last_error().contains("uninitialized"), "{}", last_error());

        // Invalid UTF-8 in setup is ignored (field unchanged).
        aria_router_setup(h, url.as_ptr(), std::ptr::null());
        let bad = unsafe { CString::from_vec_unchecked(vec![0xff, 0x00]) };
        aria_router_setup(h, bad.as_ptr(), bad.as_ptr());
        assert_eq!(
            unsafe { &*h }.base_url.as_deref(),
            Some(stub.endpoint.as_str())
        );
        assert!(unsafe { &*h }.token.is_none());
        aria_router_destroy(h);
        stub.shutdown();
    }

    struct StubChat {
        endpoint: String,
        rt: tokio::runtime::Runtime,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl StubChat {
        fn start() -> Self {
            let app = axum::Router::new()
                .route(
                    "/v1/chat/completions",
                    axum::routing::post(
                        |headers: axum::http::HeaderMap,
                         axum::Json(body): axum::Json<serde_json::Value>| async move {
                            let auth = headers
                                .get("authorization")
                                .and_then(|h| h.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            let mut resp_headers = HeaderMap::new();
                            resp_headers.insert(
                                "x-aria-router-layer",
                                HeaderValue::from_static("semantic"),
                            );
                            resp_headers.insert(
                                "x-aria-router-model",
                                HeaderValue::from_static("stub/model"),
                            );
                            resp_headers.insert(
                                "x-aria-router-redecision",
                                HeaderValue::from_static("new-user-message"),
                            );
                            // Neither non-whitelisted nor non-UTF-8 values may leak.
                            resp_headers.insert(
                                "authorization",
                                HeaderValue::from_static("must-not-leak"),
                            );
                            resp_headers.insert(
                                "x-aria-router-reason",
                                HeaderValue::from_bytes(&[0xff]).unwrap(),
                            );
                            (
                                resp_headers,
                                axum::Json(serde_json::json!({
                                    "model": body.get("model").cloned().unwrap_or(serde_json::json!("")),
                                    "auth": auth,
                                    "max_tokens": body.get("max_tokens").cloned().unwrap_or(serde_json::Value::Null),
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
            let tx = self.stop.take().expect("stop sender");
            let handle = self.handle.take().expect("server handle");
            let _ = tx.send(());
            let _ = self.rt.block_on(handle);
        }
    }

    #[test]
    fn route_headers_whitelist_only() {
        let mut headers = HeaderMap::new();
        headers.insert("x-aria-router-layer", HeaderValue::from_static("semantic"));
        headers.insert(
            "x-aria-router-redecision",
            HeaderValue::from_static("unknown"),
        );
        headers.insert("authorization", HeaderValue::from_static("secret"));
        headers.insert(
            "x-aria-router-reason",
            HeaderValue::from_bytes(&[0xff]).unwrap(),
        );
        let v = route_headers(&headers);
        assert_eq!(v["x-aria-router-layer"], "semantic");
        assert_eq!(v["x-aria-router-redecision"], "unknown");
        assert!(v.get("authorization").is_none());
        assert!(v.get("x-aria-router-reason").is_none());
        assert_eq!(route_headers(&HeaderMap::new()), serde_json::json!({}));
    }

    #[test]
    fn connected_models_and_route_headers() {
        let stub = StubChat::start();
        let url = CString::new(stub.endpoint.clone()).unwrap();
        let token = CString::new("sk-models").unwrap();
        let h = aria_router_connect(url.as_ptr());
        assert!(!h.is_null());
        aria_router_setup(h, std::ptr::null(), token.as_ptr());
        let mut buf = vec![0u8; 4096];
        let rc = aria_router_models(h, buf.as_mut_ptr() as *mut c_char, buf.len());
        let err = last_error();
        assert_eq!(rc, 0, "{err}");
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        assert!(s.contains("stub/model"), "{s}");

        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let mut out = vec![0u8; 4096];
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            std::ptr::null(),
            out.as_mut_ptr() as *mut c_char,
            out.len(),
        );
        let err = last_error();
        assert_eq!(rc, 0, "{err}");
        assert_eq!(
            aria_router_last_route(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            0
        );
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["headers"]["x-aria-router-layer"], "semantic");
        assert_eq!(v["headers"]["x-aria-router-redecision"], "new-user-message");
        assert_eq!(v["headers"]["x-aria-router-model"], "stub/model");
        assert!(v["headers"].get("authorization").is_none());
        assert!(v["headers"].get("x-aria-router-reason").is_none());
        aria_router_destroy(h);
        stub.shutdown();
    }

    fn closed_socket_endpoint() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            drop(stream);
        });
        format!("http://{addr}")
    }

    #[test]
    fn remote_failures_are_explicit() {
        let stub = StubChat::start();
        let url = CString::new(format!("{}/fail", stub.endpoint)).unwrap();
        let h = aria_router_connect(url.as_ptr());
        assert!(!h.is_null());
        let mut buf = vec![0u8; 4096];
        assert_eq!(
            aria_router_models(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            -1
        );
        let err = last_error();
        assert!(err.contains("remote status 500"), "{err}");
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        assert_eq!(
            aria_router_complete(
                h,
                msgs.as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            ),
            -1
        );
        let err = last_error();
        assert!(err.contains("remote status 500"), "{err}");
        // A failed request must not leave stale decision headers behind.
        assert_eq!(
            aria_router_last_route(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            0
        );
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_string_lossy();
        assert_eq!(s.as_ref(), "{}");
        aria_router_destroy(h);

        // accept-then-close is deterministic on every platform (no ECONNREFUSED).
        let url = CString::new(closed_socket_endpoint()).unwrap();
        let h = aria_router_connect(url.as_ptr());
        assert_eq!(
            aria_router_models(h, buf.as_mut_ptr() as *mut c_char, buf.len()),
            -1
        );
        assert!(!last_error().is_empty());
        aria_router_destroy(h);
        stub.shutdown();
    }

    #[test]
    fn complete_over_base_url_with_token_and_max_tokens() {
        let stub = StubChat::start();
        let url = CString::new(stub.endpoint.clone()).unwrap();
        let token = CString::new("sk-remote").unwrap();
        let h = aria_router_connect(url.as_ptr());
        assert!(!h.is_null());
        aria_router_setup(h, std::ptr::null(), token.as_ptr());

        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new(r#"{"model":"custom/model","max_tokens":7}"#).unwrap();
        let mut buf = vec![0u8; 4096];
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        assert_eq!(rc, 0);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_string_lossy();
        assert!(s.contains("custom/model"), "{s}");
        assert!(s.contains("Bearer sk-remote"), "{s}");
        assert!(s.contains("\"max_tokens\":7"), "{s}");

        // Empty options string defaults to `{}` (default model).
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        assert_eq!(rc, 0);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_string_lossy();
        assert!(s.contains("ariacompute/semantic-auto"), "{s}");

        // Invalid options JSON surfaces the parse error.
        let bad = CString::new("{not json").unwrap();
        assert_eq!(
            aria_router_complete(
                h,
                msgs.as_ptr(),
                bad.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            ),
            -1
        );

        // Streaming wrapper without a callback still writes the body.
        assert_eq!(
            aria_router_complete_stream(
                h,
                msgs.as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                None,
                std::ptr::null_mut(),
            ),
            0
        );
        aria_router_destroy(h);
        stub.shutdown();
    }

    #[test]
    fn init_expands_tilde() {
        // Windows CI has USERPROFILE, not HOME; match dirs::home_dir() used by resolve_home_path.
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .expect("USERPROFILE or HOME");
        let dir = home.join(".ariacompute").join("tmp").join(format!(
            "ffi-tilde-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("tilde.yaml");
        std::fs::write(
            &cfg,
            include_str!("../../config/examples/semantic-tiny.yaml"),
        )
        .unwrap();
        let rel = cfg.strip_prefix(&home).expect("under home");
        // Keep forward slashes so `~/…` parsing is stable on Windows.
        let rel_fwd = rel.to_string_lossy().replace('\\', "/");
        let tilde = format!("~/{rel_fwd}");
        let p = CString::new(tilde).unwrap();
        let h = aria_router_init(p.as_ptr());
        let _ = std::fs::remove_dir_all(&dir);
        let err = last_error();
        assert!(!h.is_null(), "{err}");
        aria_router_destroy(h);
    }

    #[test]
    fn init_ok_models() {
        let dir = tempfile_dir();
        let cfg = dir.join("c.yaml");
        std::fs::write(
            &cfg,
            include_str!("../../config/examples/semantic-tiny.yaml"),
        )
        .unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(!h.is_null());
        let mut buf = vec![0u8; 4096];
        let rc = aria_router_models(h, buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, 0);
        aria_router_destroy(h);
    }

    #[test]
    fn complete_fast_response() {
        let dir = tempfile_dir();
        let cfg = dir.join("fast-response.yaml");
        std::fs::write(
            &cfg,
            include_str!("../../bindings/testdata/fast-response.yaml"),
        )
        .unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(!h.is_null());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new(r#"{"model":"ariacompute/semantic-auto"}"#).unwrap();
        let mut buf = vec![0u8; 8192];
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        let err = last_error();
        assert_eq!(rc, 0, "{err}");
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        assert!(s.contains("hello-from-router"), "{s}");
        let mut route = vec![0u8; 4096];
        assert_eq!(
            aria_router_last_route(h, route.as_mut_ptr() as *mut c_char, route.len()),
            0
        );
        let route_json = unsafe { CStr::from_ptr(route.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        let v: serde_json::Value = serde_json::from_str(&route_json).unwrap();
        assert!(
            v["headers"]["x-aria-router-layer"].is_string(),
            "{route_json}"
        );
        assert!(v["headers"].get("authorization").is_none());
        aria_router_destroy(h);
        aria_router_destroy(std::ptr::null_mut());
    }

    #[test]
    fn complete_require_api_key_without_token() {
        let dir = tempfile_dir();
        let cfg = dir.join("fast-response-require-key.yaml");
        let mut yaml = include_str!("../../bindings/testdata/fast-response.yaml").to_string();
        yaml = yaml.replace("require_api_key: false", "require_api_key: true");
        assert!(
            yaml.contains("require_api_key: true"),
            "fixture must enable require_api_key"
        );
        std::fs::write(&cfg, yaml).unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(!h.is_null());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new(r#"{"model":"ariacompute/semantic-auto"}"#).unwrap();
        let mut buf = vec![0u8; 8192];
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        let err = last_error();
        assert_eq!(rc, 0, "{err}");
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        assert!(s.contains("hello-from-router"), "{s}");
        assert!(!s.to_lowercase().contains("unauthorized"), "{s}");
        aria_router_destroy(h);
    }

    fn init_fixture(file_name: &str, yaml: &str) -> *mut AriaRouterHandle {
        let dir = tempfile_dir();
        let cfg = dir.join(file_name);
        std::fs::write(&cfg, yaml).unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(!h.is_null(), "{}", last_error());
        h
    }

    #[test]
    fn complete_with_token_routes_through_data_plane() {
        // A configured token makes the FFI call the in-process data router with a
        // Bearer header; empty options fall back to the default model.
        let h = init_fixture(
            "fast-response.yaml",
            include_str!("../../bindings/testdata/fast-response.yaml"),
        );
        let tok = CString::new("sk-ffi-token").unwrap();
        aria_router_setup(h, std::ptr::null(), tok.as_ptr());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new("").unwrap();
        let mut buf = vec![0u8; 8192];
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        // The token routes through the in-process data plane, which rejects an
        // unknown bearer (the fake `fast response` path would have answered).
        assert_eq!(rc, -1);
        assert!(last_error().contains("invalid api key"), "{}", last_error());
        // Clearing the token (empty string) returns to the in-process SDK path.
        let empty = CString::new("").unwrap();
        aria_router_setup(h, std::ptr::null(), empty.as_ptr());
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        assert_eq!(rc, 0, "{}", last_error());
        aria_router_destroy(h);
    }

    #[test]
    fn complete_in_process_unauthorized_returns_body() {
        // A rejected key surfaces the data-plane response body as the error.
        let yaml = include_str!("../../bindings/testdata/fast-response.yaml")
            .replace("require_api_key: false", "require_api_key: true");
        let h = init_fixture("fast-response-require-key.yaml", &yaml);
        let tok = CString::new("sk-wrong").unwrap();
        aria_router_setup(h, std::ptr::null(), tok.as_ptr());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let mut buf = vec![0u8; 8192];
        let rc = aria_router_complete(
            h,
            msgs.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        assert_eq!(rc, -1);
        assert!(last_error().contains("invalid api key"), "{}", last_error());
        aria_router_destroy(h);
    }

    // The callback reports through `user` so parallel tests never share state.
    extern "C" fn stream_cb(_out: *const c_char, user: *mut libc::c_void) {
        if user.is_null() {
            return;
        }
        let called = unsafe { &*(user as *const std::sync::atomic::AtomicBool) };
        called.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn stream_callback_tolerates_null_user_data() {
        // A host may pass no user pointer; the callback must be a no-op, not UB.
        stream_cb(std::ptr::null(), std::ptr::null_mut());
    }

    #[test]
    fn complete_stream_invokes_callback_on_success() {
        let h = init_fixture(
            "fast-response.yaml",
            include_str!("../../bindings/testdata/fast-response.yaml"),
        );
        let called = std::sync::atomic::AtomicBool::new(false);
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let mut buf = vec![0u8; 8192];
        let rc = aria_router_complete_stream(
            h,
            msgs.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            Some(stream_cb),
            &called as *const _ as *mut libc::c_void,
        );
        assert_eq!(rc, 0, "{}", last_error());
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        assert!(!s.is_empty());
        aria_router_destroy(h);
    }

    #[test]
    fn complete_stream_failure_skips_callback() {
        let h = init_fixture(
            "fast-response.yaml",
            include_str!("../../bindings/testdata/fast-response.yaml"),
        );
        let called = std::sync::atomic::AtomicBool::new(false);
        let bad = CString::new("{not json").unwrap();
        let mut buf = vec![0u8; 1024];
        let rc = aria_router_complete_stream(
            h,
            bad.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            Some(stream_cb),
            &called as *const _ as *mut libc::c_void,
        );
        assert_eq!(rc, -1);
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        aria_router_destroy(h);
    }

    #[test]
    fn setup_memory_only() {
        let dir = tempfile_dir();
        let cfg = dir.join("fast-response.yaml");
        std::fs::write(
            &cfg,
            include_str!("../../bindings/testdata/fast-response.yaml"),
        )
        .unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(!h.is_null());
        let tok = CString::new("sk-test").unwrap();
        aria_router_setup(h, std::ptr::null(), tok.as_ptr());
        assert_eq!(unsafe { &*h }.token.as_deref(), Some("sk-test"));
        let empty = CString::new("").unwrap();
        aria_router_setup(h, std::ptr::null(), empty.as_ptr());
        assert!(unsafe { &*h }.token.is_none());
        aria_router_destroy(h);
    }

    #[test]
    fn complete_stream_callback() {
        let dir = tempfile_dir();
        let cfg = dir.join("fast-response.yaml");
        std::fs::write(
            &cfg,
            include_str!("../../bindings/testdata/fast-response.yaml"),
        )
        .unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = aria_router_init(p.as_ptr());
        assert!(!h.is_null());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new(r#"{"model":"ariacompute/semantic-auto"}"#).unwrap();
        let mut buf = vec![0u8; 8192];
        extern "C" fn cb(_chunk: *const c_char, hits: *mut libc::c_void) {
            unsafe {
                *(hits as *mut i32) += 1;
            }
        }
        let mut hits: i32 = 0;
        let rc = aria_router_complete_stream(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            Some(cb),
            &mut hits as *mut i32 as *mut libc::c_void,
        );
        assert_eq!(rc, 0);
        assert!(hits >= 1);
        aria_router_destroy(h);
    }

    extern "C" fn collect_cb(chunk: *const c_char, user: *mut libc::c_void) {
        let out = unsafe { &mut *(user as *mut Vec<String>) };
        out.push(
            unsafe { CStr::from_ptr(chunk) }
                .to_string_lossy()
                .into_owned(),
        );
    }

    #[test]
    fn complete_stream_callback_receives_full_body() {
        let h = init_fixture(
            "fast-response.yaml",
            include_str!("../../bindings/testdata/fast-response.yaml"),
        );
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let mut buf = vec![0u8; 8192];
        let mut chunks: Vec<String> = Vec::new();
        let rc = aria_router_complete_stream(
            h,
            msgs.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            Some(collect_cb),
            &mut chunks as *mut Vec<String> as *mut libc::c_void,
        );
        assert_eq!(rc, 0, "{}", last_error());
        let body = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(chunks, vec![body.clone()]);
        assert!(body.contains("hello-from-router"), "{body}");
        aria_router_destroy(h);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "aria-router-ffi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
