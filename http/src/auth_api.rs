//! Management auth, users, and OAuth account HTTP handlers.

use crate::keys::ExchangeInput;
use crate::serve_account::{ServeAccountPublic, ServeUserInfo};
use crate::users::{extract_session_token, UserPublic, UserRole};
use crate::AppError;
use crate::AppState;
use aria_router_core::RouterError;
use axum::extract::{Query, State};
use reqwest::Method;
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

#[derive(Deserialize)]
pub struct EmailBody {
    pub email: Option<String>,
}

/// Set the current user's email (used as the serve OAuth account on link).
/// Empty string or null clears it.
pub async fn set_my_email(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    st.users
        .lock()
        .unwrap()
        .set_email(&u.id, body.email)
        .map_err(AppError)?;
    Ok(Json(json!({"ok": true})))
}

/// Admin: set another user's email.
pub async fn set_user_email(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, AppError> {
    let u = require_user(&st, &headers).map_err(AppError)?;
    if !matches!(u.role, UserRole::Admin) {
        return Err(AppError(RouterError::Unauthorized("admin required".into())));
    }
    st.users
        .lock()
        .unwrap()
        .set_email(&id, body.email)
        .map_err(AppError)?;
    Ok(Json(json!({"ok": true, "id": id})))
}

pub async fn serve_account_get(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ServeAccountPublic>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    Ok(Json(st.keys.lock().unwrap().oauth_public()))
}

#[derive(Deserialize)]
pub struct OAuthStartBody {
    #[serde(default)]
    pub site: Option<String>,
}

/// Public OAuth login entry point (no session required). Starts the Aria
/// Compute (serve) handshake and returns the authorize URL. The browser is
/// redirected to serve; after the user authenticates, serve calls back to
/// `/v1/router/auth/oauth/callback`, which upserts the serve identity as a
/// router dashboard user and issues a session.
pub async fn oauth_start(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<OAuthStartBody>,
) -> Result<Json<Value>, AppError> {
    let site = body
        .site
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| st.doc.lock().unwrap().global.serve_site.clone());
    let (tpl, state, site_url) = st
        .keys
        .lock()
        .unwrap()
        .begin_link(&site, None, None, None)
        .map_err(AppError)?;
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:8080");
    let callback = format!("http://{host}/v1/router/auth/oauth/callback");
    let authorize_url = tpl.replace("{callback}", &urlencoding_encode(&callback));
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

pub async fn oauth_callback(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LinkCallbackQ>,
) -> Response {
    if let Some(err) = q.error {
        return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&err))).into_response();
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return Redirect::temporary("/?error=missing_code").into_response();
    };
    let (site, site_url, _owner_user_id, _owner_email) = match st
        .keys
        .lock()
        .unwrap()
        .take_pending(&state)
    {
        Ok(v) => v,
        Err(e) => {
            return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    let exchange_url = format!("{}/api/router-link/exchange", site_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build() {
        Ok(c) => c,
        Err(e) => {
            return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
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
                return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
                    .into_response();
            }
        },
        Err(e) => {
            return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    if body.get("error").is_some() {
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("exchange_failed");
        return Redirect::temporary(&format!("/?error={}", urlencoding_encode(msg)))
            .into_response();
    }
    // The serve exchange resolves (and, if needed, creates) the serve oauth
    // user. Use its identity to upsert a router dashboard user + decide role.
    let serve_user = body.get("user").and_then(|v| v.as_object()).cloned();
    let serve_id = serve_user
        .as_ref()
        .and_then(|u| u.get("id"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let serve_id_str = match &serve_id {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return Redirect::temporary("/?error=missing_serve_id").into_response(),
    };
    let serve_email = serve_user
        .as_ref()
        .and_then(|u| u.get("email"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let serve_name = serve_user
        .as_ref()
        .and_then(|u| u.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let serve_role = serve_user
        .as_ref()
        .and_then(|u| u.get("role"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let display = ServeUserInfo {
        id: serve_id.clone(),
        email: serve_email.clone(),
        role: serve_role,
    };
    // Upsert the serve identity as a router dashboard user (role by whitelist /
    // first-admin bootstrap). The serve role is intentionally NOT mirrored.
    let admin_emails = st.doc.lock().unwrap().global.admin_emails.clone();
    let user = match st
        .users
        .lock()
        .unwrap()
        .upsert_serve_user(&serve_id_str, serve_email.clone(), serve_name, &admin_emails)
    {
        Ok(u) => u,
        Err(e) => {
            return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    // Store the serve account globally (api key / link_token) for LLM proxying.
    let link_token = body
        .get("link_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expires_at = body
        .get("expires_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // Link the serve account. The router does NOT create a serve API key on the
    // user's behalf; the oauth user creates their own key on serve, the dashboard
    // auto-syncs its metadata, and the user pastes the bfvk- plaintext once.
    let site_label = body.get("site").and_then(|v| v.as_str()).unwrap_or(&site);

    let mut keys_guard = st.keys.lock().unwrap();
    // Re-linking a different serve account: drop the previously pasted key so it
    // is not reused for the new account.
    let switching = keys_guard
        .oauth_owner_user_id()
        .map(|o| o != user.id)
        .unwrap_or(false);
    if switching {
        let _ = keys_guard.oauth_clear_api_key();
    }
    let exchange = ExchangeInput {
        site: site_label.to_string(),
        site_url: site_url.clone(),
        user: display,
        link_token: link_token.clone(),
        expires_at,
        api_key: None,
        owner_user_id: Some(user.id.clone()),
    };
    if let Err(e) = keys_guard.apply_exchange(exchange) {
        return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
            .into_response();
    }
    drop(keys_guard);
    // Issue a router dashboard session for the upserted user.
    let token = match st.users.lock().unwrap().issue_session(&user.id) {
        Ok(t) => t,
        Err(e) => {
            return Redirect::temporary(&format!("/?error={}", urlencoding_encode(&e.to_string())))
                .into_response();
        }
    };
    // Best-effort: auto-sync the serve key metadata from serve (display only). The
    // secret is not exposed by serve's list endpoint and is pasted by the user.
    // Run in a detached task so the handler future stays `Send` (no borrow held
    // across an await here); sync failures are non-fatal for the link flow.
    if let Some(lt) = link_token.clone() {
        let st_task = Arc::clone(&st);
        let site_url_task = site_url.clone();
        tokio::spawn(async move {
            if let Ok(list) = serve_fetch_api_keys(&site_url_task, &lt).await {
                if let Some((name, prefix)) = serve_pick_api_key(&list) {
                    let _ = st_task
                        .keys
                        .lock()
                        .unwrap()
                        .oauth_set_api_key_meta(name, prefix);
                }
            }
        });
    }
    let mut res = Redirect::temporary("/?oauth=1").into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&token).parse().unwrap());
    res
}

/// Call a serve JSON API endpoint authenticated with a Bearer token.
async fn serve_json(
    method: Method,
    url: &str,
    bearer: &str,
    body: Option<Value>,
) -> Result<Value, RouterError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| RouterError::Upstream(format!("serve client: {e}")))?;
    let mut req = client
        .request(method, url)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    if let Some(b) = body {
        req = req.header(header::CONTENT_TYPE, "application/json").json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| RouterError::Upstream(format!("serve request to {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(RouterError::Upstream(format!(
            "serve {url}: status {}",
            resp.status()
        )));
    }
    resp.json()
        .await
        .map_err(|e| RouterError::Upstream(format!("serve json from {url}: {e}")))
}

/// Fetch the linked serve account's API keys (metadata only) from serve using a
/// bearer credential (the stored bfvk- after link, or the short-lived link_token at
/// link time). The secret is never returned by serve's list endpoint.
async fn serve_fetch_api_keys(
    site_url: &str,
    bearer: &str,
) -> Result<Vec<Value>, RouterError> {
    let url = format!("{}/api/api-keys", site_url.trim_end_matches('/'));
    let v = serve_json(Method::GET, &url, bearer, None).await?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// Pick the serve API key to surface on the router dashboard: the most recently
/// created active key. Its secret must be pasted by the user separately (serve's
/// list endpoint never returns it).
fn serve_pick_api_key(list: &[Value]) -> Option<(String, String)> {
    let mut best: Option<&Value> = None;
    for m in list {
        match best {
            Some(b) => {
                let bt = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                let mt = m.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                if mt > bt {
                    best = Some(m);
                }
            }
            None => best = Some(m),
        }
    }
    let m = best?;
    let name = m
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("aria-router")
        .to_string();
    let prefix = m
        .get("prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((name, prefix))
}

/// Re-sync the linked serve account's API key metadata from serve. Prefers the
/// stored serve API key (bfvk-) as the credential and falls back to the
/// (short-lived) link token. Returns the updated public serve account.
pub async fn serve_account_sync(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ServeAccountPublic>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    let (site_url, bearer) = {
        let keys = st.keys.lock().unwrap();
        let acct = keys.oauth_public();
        let url = acct
            .site_url
            .clone()
            .ok_or_else(|| RouterError::InvalidParam("not linked to Aria Compute".into()))?;
        let cred = keys.oauth_api_key().or_else(|| keys.oauth_link_token());
        (url, cred)
    };
    let bearer = bearer.ok_or_else(|| {
        RouterError::InvalidParam(
            "no serve credential available; re-link your Aria Compute account".into(),
        )
    })?;
    let list_url = format!("{}/api/api-keys", site_url.trim_end_matches('/'));
    let metas = serve_json(Method::GET, &list_url, &bearer, None)
        .await
        .map_err(AppError)?;
    let arr = metas.as_array().cloned().unwrap_or_default();
    let stored_prefix = st.keys.lock().unwrap().oauth_public().api_key_prefix.clone();
    let chosen = arr
        .iter()
        .find(|m| {
            stored_prefix
                .as_deref()
                .map(|p| m.get("prefix").and_then(|x| x.as_str()) == Some(p))
                .unwrap_or(false)
        })
        .or_else(|| arr.first());
    if let Some(m) = chosen {
        let name = m
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("aria-router")
            .to_string();
        let prefix = m
            .get("prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        st.keys
            .lock()
            .unwrap()
            .oauth_set_api_key_meta(name, prefix)
            .map_err(AppError)?;
    }
    Ok(Json(st.keys.lock().unwrap().oauth_public()))
}

/// Store the serve API key (bfvk-) the user created on serve, so the router can
/// use it as a Bearer credential and call back into serve after the link token
/// expires. Serve's list endpoint never returns the secret, so the user pastes it
/// once; the key's name/prefix are synced separately via `serve_account_sync`.
pub async fn serve_account_set_key(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<ServeAccountPublic>, AppError> {
    let _ = gate_if_users(&st, &headers).map_err(AppError)?;
    let raw = body
        .get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError(RouterError::InvalidParam("api_key required".into())))?;
    let mut keys = st.keys.lock().unwrap();
    if !keys.oauth_public().linked {
        return Err(AppError(RouterError::InvalidParam(
            "no linked serve account".into(),
        )));
    }
    keys.oauth_set_api_key(raw, None).map_err(AppError)?;
    Ok(Json(keys.oauth_public()))
}
