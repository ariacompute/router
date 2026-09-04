//! Management auth, users, and OAuth account HTTP handlers.

use crate::serve_account::{ServeAccountPublic, ServeUserInfo};
use crate::users::{extract_session_token, UserPublic, UserRole};
use crate::AppError;
use crate::AppState;
use aria_router_core::RouterError;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn session_cookie(token: &str) -> String {
    format!("aria_router_session={token}; Path=/; HttpOnly; SameSite=Lax")
}

pub fn clear_session_cookie() -> String {
    "aria_router_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".into()
}

pub fn require_user(st: &AppState, headers: &HeaderMap) -> Result<UserPublic, RouterError> {
    let Some(tok) = extract_session_token(headers) else {
        return Err(RouterError::Unauthorized("login required".into()));
    };
    st.users.lock().unwrap().resolve_session(&tok)
}

pub fn optional_user(st: &AppState, headers: &HeaderMap) -> Option<UserPublic> {
    require_user(st, headers).ok()
}

pub fn gate_if_users(st: &AppState, headers: &HeaderMap) -> Result<Option<UserPublic>, RouterError> {
    if st.users.lock().unwrap().is_empty() {
        return Ok(None);
    }
    Ok(Some(require_user(st, headers)?))
}

#[derive(Deserialize)]
pub struct AuthBody {
    pub username: String,
    pub password: String,
}

pub async fn register_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let empty = st.users.lock().unwrap().is_empty();
    let allow = st.doc.lock().unwrap().global.allow_register;
    Json(json!({
        "allow_register": allow && !empty,
        "needs_setup": empty,
    }))
}

pub async fn register(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AuthBody>,
) -> Response {
    if st.users.lock().unwrap().is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "no local users yet; run aria-router setup first"})),
        )
            .into_response();
    }
    let allow = st.doc.lock().unwrap().global.allow_register;
    let result = st
        .users
        .lock()
        .unwrap()
        .register(&body.username, &body.password, allow);
    match result {
        Ok((user, token)) => {
            let mut res = Json(json!({"user": user, "token": token})).into_response();
            res.headers_mut().insert(
                header::SET_COOKIE,
                session_cookie(&token).parse().unwrap(),
            );
            res
        }
        Err(e) => AppError(e).into_response(),
    }
}

pub async fn login(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AuthBody>,
) -> Result<Response, AppError> {
    let (user, token) = st
        .users
        .lock()
        .unwrap()
        .login(&body.username, &body.password)
        .map_err(AppError)?;
    let mut res = Json(json!({"user": user, "token": token})).into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&token).parse().unwrap(),
    );
    Ok(res)
}

pub async fn logout(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(tok) = extract_session_token(&headers) {
        st.users.lock().unwrap().logout(&tok);
    }
    let mut res = Json(json!({"ok": true})).into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        clear_session_cookie().parse().unwrap(),
    );
    res
}

pub async fn me(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    Ok(Json(json!({"user": u})))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

pub async fn change_password(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PasswordBody>,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    st.users
        .lock()
        .unwrap()
        .set_password(&u.id, &body.password)
        .map_err(AppError)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn list_users(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    if !matches!(u.role, UserRole::Admin) {
        return Err(AppError(RouterError::Unauthorized("admin required".into())));
    }
    Ok(Json(json!({"users": st.users.lock().unwrap().list_public()})))
}

#[derive(Deserialize)]
pub struct DisableBody {
    pub disabled: bool,
}

pub async fn set_user_disabled(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<DisableBody>,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    if !matches!(u.role, UserRole::Admin) {
        return Err(AppError(RouterError::Unauthorized("admin required".into())));
    }
    st.users
        .lock()
        .unwrap()
        .set_disabled(&id, body.disabled)
        .map_err(AppError)?;
    Ok(Json(json!({"ok": true, "id": id})))
}

#[derive(Deserialize)]
pub struct AllowRegisterBody {
    pub allow_register: bool,
}

pub async fn set_allow_register(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AllowRegisterBody>,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    if !matches!(u.role, UserRole::Admin) {
        return Err(AppError(RouterError::Unauthorized("admin required".into())));
    }
    st.doc.lock().unwrap().global.allow_register = body.allow_register;
    Ok(Json(json!({"ok": true, "allow_register": body.allow_register})))
}

pub async fn serve_account_get(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ServeAccountPublic>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    Ok(Json(st.keys.lock().unwrap().oauth_public()))
}

pub async fn serve_account_secret(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    let secret = st.keys.lock().unwrap().oauth_reveal_secret();
    Ok(Json(json!({"api_key": secret})))
}

pub async fn serve_account_delete(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    st.keys.lock().unwrap().oauth_clear().map_err(AppError)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct LinkStartBody {
    pub site: String,
}

pub async fn serve_link_start(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<LinkStartBody>,
) -> Result<Json<Value>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    let (tpl, state, site_url) = st
        .keys
        .lock()
        .unwrap()
        .begin_link(&body.site)
        .map_err(AppError)?;
    // Caller substitutes callback; we also return a ready URL if Host present.
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:8080");
    let callback = format!("http://{host}/v1/router/serve/link/callback");
    let authorize_url = tpl.replace("{callback}", &urlencoding_encode(&callback));
    let _ = st.keys.lock().unwrap().oauth_set_site(&body.site);
    Ok(Json(json!({
        "authorize_url": authorize_url,
        "state": state,
        "site_url": site_url,
        "callback": callback,
    })))
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Deserialize)]
pub struct LinkCallbackQ {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub async fn serve_link_callback(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LinkCallbackQ>,
) -> Response {
    if let Some(err) = q.error {
        return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(&err)))
            .into_response();
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return Redirect::temporary("/account?error=missing_code").into_response();
    };
    let (site, site_url) = match st.keys.lock().unwrap().take_pending(&state) {
        Ok(v) => v,
        Err(e) => {
            return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    let exchange_url = format!("{}/api/router-link/exchange", site_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build() {
        Ok(c) => c,
        Err(e) => {
            return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    let resp = client
        .post(&exchange_url)
        .json(&json!({"code": code}))
        .send()
        .await;
    let body: Value = match resp {
        Ok(r) => match r.json().await {
            Ok(v) => v,
            Err(e) => {
                return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(&e.to_string())))
                    .into_response();
            }
        },
        Err(e) => {
            return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    if body.get("error").is_some() {
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("exchange_failed");
        return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(msg)))
            .into_response();
    }
    let user = ServeUserInfo {
        id: body
            .pointer("/user/id")
            .cloned()
            .unwrap_or(json!(null)),
        email: body
            .pointer("/user/email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        role: body
            .pointer("/user/role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };
    let link_token = body
        .get("link_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expires_at = body
        .get("expires_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let site_label = body
        .get("site")
        .and_then(|v| v.as_str())
        .unwrap_or(&site);
    if let Err(e) = st.keys.lock().unwrap().apply_exchange(
        site_label,
        &site_url,
        user,
        link_token,
        expires_at,
        None,
    ) {
        return Redirect::temporary(&format!("/account?error={}", urlencoding_encode(&e.to_string())))
            .into_response();
    }
    Redirect::temporary("/account?linked=1").into_response()
}

#[derive(Deserialize)]
pub struct ServeApiKeyBody {
    pub api_key: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub site: Option<String>,
}

pub async fn serve_put_api_key(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ServeApiKeyBody>,
) -> Result<Json<Value>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    let mut keys = st.keys.lock().unwrap();
    if let Some(site) = body.site.as_deref() {
        keys.oauth_set_site(site).map_err(AppError)?;
    }
    keys.oauth_set_api_key(&body.api_key, body.name.as_deref())
        .map_err(AppError)?;
    Ok(Json(json!({"ok": true, "account": keys.oauth_public()})))
}

#[derive(Deserialize)]
pub struct CreateServeKeyBody {
    #[serde(default = "default_key_name")]
    pub name: String,
}

fn default_key_name() -> String {
    "aria-router".into()
}

pub async fn serve_create_api_key(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateServeKeyBody>,
) -> Result<Json<Value>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    let (site_url, token) = {
        let keys = st.keys.lock().unwrap();
        (
            keys.oauth_site_url().ok_or_else(|| {
                AppError(RouterError::InvalidParam(
                    "link OAuth account or set site first".into(),
                ))
            })?,
            keys.oauth_link_token().ok_or_else(|| {
                AppError(RouterError::Unauthorized(
                    "OAuth link_token missing; re-link account".into(),
                ))
            })?,
        )
    };
    let url = format!("{}/api/api-keys", site_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError(RouterError::Upstream(e.to_string())))?;
    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({"name": body.name}))
        .send()
        .await
        .map_err(|e| AppError(RouterError::Upstream(e.to_string())))?;
    if !resp.status().is_success() {
        let t = resp.text().await.unwrap_or_default();
        return Err(AppError(RouterError::Upstream(format!(
            "create api key failed: {t}"
        ))));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| AppError(RouterError::Upstream(e.to_string())))?;
    let secret = v
        .get("key")
        .or_else(|| v.get("value"))
        .or_else(|| v.get("api_key"))
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            AppError(RouterError::Upstream(
                "create api key response missing secret".into(),
            ))
        })?;
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or(&body.name);
    st.keys
        .lock()
        .unwrap()
        .oauth_set_api_key(secret, Some(name))
        .map_err(AppError)?;
    Ok(Json(json!({
        "ok": true,
        "api_key": secret,
        "account": st.keys.lock().unwrap().oauth_public(),
    })))
}
