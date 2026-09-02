//! C ABI for ariarouter (`libariarouter_ffi`).

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI: pointers are caller-owned

use ariarouter_config::RouterDocument;
use ariarouter_http::{data_router, last_route_json, AppState};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

thread_local! {
    static LAST_ERROR: Mutex<Option<CString>> = const { Mutex::new(None) };
}

fn set_err(msg: &str) {
    LAST_ERROR.with(|s| {
        *s.lock().unwrap() = Some(CString::new(msg).unwrap_or_else(|_| CString::new("err").unwrap()));
    });
}

#[no_mangle]
pub extern "C" fn ariarouter_last_error() -> *const c_char {
    LAST_ERROR.with(|s| {
        s.lock()
            .unwrap()
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

pub struct AriarouterHandle {
    state: Option<Arc<AppState>>,
    base_url: Option<String>,
    rt: tokio::runtime::Runtime,
}

impl AriarouterHandle {
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("rt")
    }
}

fn cstr<'a>(p: *const c_char) -> Result<&'a str, ()> {
    if p.is_null() {
        return Err(());
    }
    unsafe { CStr::from_ptr(p).to_str().map_err(|_| ()) }
}

#[no_mangle]
pub extern "C" fn ariarouter_init(config_path: *const c_char) -> *mut AriarouterHandle {
    let path = match cstr(config_path) {
        Ok(s) => s,
        Err(()) => {
            set_err("null config_path");
            return std::ptr::null_mut();
        }
    };
    match RouterDocument::load_path(path) {
        Ok(doc) => {
            let h = Box::new(AriarouterHandle {
                state: Some(Arc::new(AppState::new(doc))),
                base_url: None,
                rt: AriarouterHandle::runtime(),
            });
            Box::into_raw(h)
        }
        Err(e) => {
            set_err(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn ariarouter_connect(base_url: *const c_char) -> *mut AriarouterHandle {
    let url = match cstr(base_url) {
        Ok(s) => s.to_string(),
        Err(()) => {
            set_err("null base_url");
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(AriarouterHandle {
        state: None,
        base_url: Some(url),
        rt: AriarouterHandle::runtime(),
    }))
}

#[no_mangle]
pub extern "C" fn ariarouter_destroy(router: *mut AriarouterHandle) {
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

fn complete_inner(h: &AriarouterHandle, messages_json: &str, options_json: &str) -> Result<String, String> {
    let opts: serde_json::Value =
        serde_json::from_str(if options_json.is_empty() { "{}" } else { options_json })
            .map_err(|e| e.to_string())?;
    let model = opts
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("aria/semantic-auto");
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
        let app = data_router(st.clone());
        let body = serde_json::to_vec(&req_json).unwrap();
        let resp = h.rt.block_on(async {
            app.oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
        })
        .map_err(|e| e.to_string())?;
        let status = resp.status();
        let bytes = h
            .rt
            .block_on(to_bytes(resp.into_body(), 1 << 22))
            .map_err(|e| e.to_string())?;
        if status != StatusCode::OK {
            return Err(String::from_utf8_lossy(&bytes).into_owned());
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    } else if let Some(url) = &h.base_url {
        let url = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
        let resp = h
            .rt
            .block_on(async {
                reqwest::Client::new().post(&url).json(&req_json).send().await
            })
            .map_err(|e| e.to_string())?;
        let text = h
            .rt
            .block_on(resp.text())
            .map_err(|e| e.to_string())?;
        Ok(text)
    } else {
        Err("uninitialized".into())
    }
}

#[no_mangle]
pub extern "C" fn ariarouter_complete(
    router: *mut AriarouterHandle,
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
pub extern "C" fn ariarouter_complete_stream(
    router: *mut AriarouterHandle,
    messages_json: *const c_char,
    options_json: *const c_char,
    out: *mut c_char,
    out_len: usize,
    callback: Option<extern "C" fn(*const c_char, *mut libc::c_void)>,
    user_data: *mut libc::c_void,
) -> i32 {
    let rc = ariarouter_complete(router, messages_json, options_json, out, out_len);
    if rc == 0 {
        if let Some(cb) = callback {
            cb(out, user_data);
        }
    }
    rc
}

#[no_mangle]
pub extern "C" fn ariarouter_models(
    router: *mut AriarouterHandle,
    out: *mut c_char,
    out_len: usize,
) -> i32 {
    if router.is_null() {
        set_err("null router");
        return -1;
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
    write_out(out, out_len, "{\"data\":[]}")
}

#[no_mangle]
pub extern "C" fn ariarouter_last_route(
    router: *mut AriarouterHandle,
    out: *mut c_char,
    out_len: usize,
) -> i32 {
    if router.is_null() {
        set_err("null router");
        return -1;
    }
    let h = unsafe { &*router };
    if let Some(st) = &h.state {
        let v = last_route_json(st);
        return write_out(out, out_len, &v.to_string());
    }
    write_out(out, out_len, "{}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn init_missing() {
        let p = CString::new("/no/such/config.yaml").unwrap();
        let h = ariarouter_init(p.as_ptr());
        assert!(h.is_null());
        assert!(!ariarouter_last_error().is_null());
    }

    #[test]
    fn init_ok_models() {
        let dir = tempfile_dir();
        let cfg = dir.join("c.yaml");
        std::fs::write(&cfg, include_str!("../../config/examples/semantic-tiny.yaml")).unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = ariarouter_init(p.as_ptr());
        assert!(!h.is_null());
        let mut buf = vec![0u8; 4096];
        let rc = ariarouter_models(h, buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, 0);
        ariarouter_destroy(h);
    }

    #[test]
    fn complete_fast_response() {
        let dir = tempfile_dir();
        let cfg = dir.join("ffi.yaml");
        std::fs::write(&cfg, include_str!("../../config/examples/ffi-tiny.yaml")).unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = ariarouter_init(p.as_ptr());
        assert!(!h.is_null());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new(r#"{"model":"aria/semantic-auto"}"#).unwrap();
        let mut buf = vec![0u8; 8192];
        let rc = ariarouter_complete(
            h,
            msgs.as_ptr(),
            opts.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        assert_eq!(rc, 0, "{}", unsafe {
            CStr::from_ptr(ariarouter_last_error()).to_string_lossy()
        });
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        assert!(s.contains("hello-from-router"), "{s}");
        let mut route = vec![0u8; 4096];
        assert_eq!(
            ariarouter_last_route(h, route.as_mut_ptr() as *mut c_char, route.len()),
            0
        );
        ariarouter_destroy(h);
        ariarouter_destroy(std::ptr::null_mut());
    }

    #[test]
    fn complete_stream_callback() {
        let dir = tempfile_dir();
        let cfg = dir.join("ffi.yaml");
        std::fs::write(&cfg, include_str!("../../config/examples/ffi-tiny.yaml")).unwrap();
        let p = CString::new(cfg.to_str().unwrap()).unwrap();
        let h = ariarouter_init(p.as_ptr());
        assert!(!h.is_null());
        let msgs = CString::new(r#"[{"role":"user","content":"hi"}]"#).unwrap();
        let opts = CString::new(r#"{"model":"aria/semantic-auto"}"#).unwrap();
        let mut buf = vec![0u8; 8192];
        extern "C" fn cb(_chunk: *const c_char, hits: *mut libc::c_void) {
            unsafe {
                *(hits as *mut i32) += 1;
            }
        }
        let mut hits: i32 = 0;
        let rc = ariarouter_complete_stream(
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
        ariarouter_destroy(h);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ariarouter-ffi-{}-{}",
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
