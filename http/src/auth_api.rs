//! Management auth, users, and OAuth account HTTP handlers.

use crate::keys::ExchangeInput;
use crate::serve_account::{ServeAccountPublic, ServeUserInfo};
use crate::users::{extract_session_token, UserPublic, UserRole};
use crate::AppError;
use crate::AppState;
use aria_router_core::RouterError;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use reqwest::Method;
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

pub fn gate_if_users(
    st: &AppState,
    headers: &HeaderMap,
) -> Result<Option<UserPublic>, RouterError> {
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

pub async fn register(State(st): State<Arc<AppState>>, Json(body): Json<AuthBody>) -> Response {
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
            res.headers_mut()
                .insert(header::SET_COOKIE, session_cookie(&token).parse().unwrap());
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
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&token).parse().unwrap());
    Ok(res)
}

pub async fn logout(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(tok) = extract_session_token(&headers) {
        st.users.lock().unwrap().logout(&tok);
    }
    let mut res = Json(json!({"ok": true})).into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, clear_session_cookie().parse().unwrap());
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
    Ok(Json(
        json!({"users": st.users.lock().unwrap().list_public()}),
    ))
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
    Ok(Json(
        json!({"ok": true, "allow_register": body.allow_register}),
    ))
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

/// Redirect back to the dashboard login page with a URL-encoded error reason.
fn oauth_error_redirect(msg: &str) -> Response {
    Redirect::temporary(&format!("/?error={}", urlencoding_encode(msg))).into_response()
}

pub async fn oauth_callback(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LinkCallbackQ>,
) -> Response {
    // Failures redirect back to the dashboard with `?error=<reason>`; the inner
    // function models that as `Err(response)` so every step stays `?`-shaped.
    oauth_callback_inner(&st, q).await.unwrap_or_else(|res| res)
}

#[allow(clippy::result_large_err)] // one redirect response per failure arm
async fn oauth_callback_inner(st: &Arc<AppState>, q: LinkCallbackQ) -> Result<Response, Response> {
    if let Some(err) = q.error {
        return Err(oauth_error_redirect(&err));
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return Err(oauth_error_redirect("missing_code"));
    };
    let (site, site_url, _owner_user_id, _owner_email) = st
        .keys
        .lock()
        .unwrap()
        .take_pending(&state)
        .map_err(|e| oauth_error_redirect(&e.to_string()))?;
    let exchange_url = format!(
        "{}/api/router-link/exchange",
        site_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| oauth_error_redirect(&e.to_string()))?;
    let body: Value = client
        .post(&exchange_url)
        .json(&json!({"code": code}))
        .send()
        .await
        .map_err(|e| oauth_error_redirect(&e.to_string()))?
        .json()
        .await
        .map_err(|e| oauth_error_redirect(&e.to_string()))?;
    if body.get("error").is_some() {
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("exchange_failed");
        return Err(oauth_error_redirect(msg));
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
        _ => return Err(oauth_error_redirect("missing_serve_id")),
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
    let user = st
        .users
        .lock()
        .unwrap()
        .upsert_serve_user(
            &serve_id_str,
            serve_email.clone(),
            serve_name,
            &admin_emails,
        )
        .map_err(|e| oauth_error_redirect(&e.to_string()))?;
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
    // auto-syncs its metadata, and the user pastes the sk-bf- plaintext once.
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
    keys_guard
        .apply_exchange(exchange)
        .map_err(|e| oauth_error_redirect(&e.to_string()))?;
    drop(keys_guard);
    // Issue a router dashboard session for the upserted user.
    let token = st
        .users
        .lock()
        .unwrap()
        .issue_session(&user.id)
        .map_err(|e| oauth_error_redirect(&e.to_string()))?;
    // Best-effort: auto-sync the serve API key from serve. Serve now returns the
    // plaintext secret in the key list, so store it directly as the durable Bearer
    // credential — no manual paste needed. Run in a detached task so the handler
    // future stays `Send` (no borrow held across an await here); sync failures are
    // non-fatal for the link flow.
    if let Some(lt) = link_token.clone() {
        tokio::spawn(sync_serve_key(Arc::clone(st), site_url.clone(), lt));
    }
    let mut res = Redirect::temporary("/?oauth=1").into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&token).parse().unwrap());
    Ok(res)
}

/// Best-effort sync of the serve API key after an OAuth link.
///
/// Kept as a free function so the detached task's body (including the failure
/// path) is directly testable without racing a spawned task.
async fn sync_serve_key(st: Arc<AppState>, site_url: String, link_token: String) {
    if let Ok(list) = serve_fetch_api_keys(&site_url, &link_token).await {
        if let Some((name, _prefix, secret)) = serve_pick_api_key(&list) {
            let _ = st
                .keys
                .lock()
                .unwrap()
                .oauth_set_api_key(&secret, Some(&name));
        }
    }
}

/// Call a serve JSON API endpoint authenticated with a Bearer token.
async fn serve_json(url: &str, bearer: &str) -> Result<Value, RouterError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| RouterError::Upstream(format!("serve client: {e}")))?;
    let req = client
        .request(Method::GET, url)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"));
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

/// Fetch the linked serve account's API keys from serve using a bearer credential
/// (the stored sk-bf- after link, or the short-lived link_token at link time). Serve
/// now returns the plaintext secret (`sk-bf-`) in each key object so the router can
/// auto-sync it without a manual paste.
async fn serve_fetch_api_keys(site_url: &str, bearer: &str) -> Result<Vec<Value>, RouterError> {
    let url = format!("{}/api/api-keys", site_url.trim_end_matches('/'));
    let v = serve_json(&url, bearer).await?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// Pick the serve API key to surface on the router dashboard: the most recently
/// created active key. Returns `(name, prefix, secret)` — serve now returns the
/// plaintext secret in the list, so the router can store it as the durable Bearer
/// credential and skip the manual paste entirely. Returns `None` when no usable key
/// (empty list, or a key without a secret) is present.
fn serve_pick_api_key(list: &[Value]) -> Option<(String, String, String)> {
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
    // Serve returns the plaintext secret now; the router stores it directly.
    let secret = m
        .get("secret")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if secret.is_empty() {
        return None;
    }
    Some((name, prefix, secret))
}

/// Re-sync the linked serve account's API key metadata from serve. Prefers the
/// stored serve API key (sk-bf-) as the credential and falls back to the
/// (short-lived) link token. Returns the updated public serve account.
///
/// Deletion/revocation detection: if listing keys with the stored key returns
/// 401/403, or the stored key's prefix is absent from the returned list, the
/// key was deleted/revoked on serve. We then mark it deleted (clearing the
/// stale secret) so the dashboard updates automatically and the router stops
/// authenticating with a dead credential. Network/timeout failures leave the
/// prior state untouched.
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| RouterError::Upstream(format!("serve client: {e}")))?;
    let resp = client
        .get(&list_url)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .send()
        .await
        .map_err(|e| RouterError::Upstream(format!("serve request to {list_url}: {e}")))?;
    let status = resp.status();
    // A 401/403 with a stored (configured) key means the key was deleted or
    // revoked on serve. Mark it so the dashboard updates automatically.
    if (status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN)
        && st.keys.lock().unwrap().oauth_api_key().is_some()
    {
        st.keys
            .lock()
            .unwrap()
            .oauth_mark_api_key_deleted()
            .map_err(AppError)?;
        return Ok(Json(st.keys.lock().unwrap().oauth_public()));
    }
    if !status.is_success() {
        return Err(AppError(RouterError::Upstream(format!(
            "serve {list_url}: status {status}"
        ))));
    }
    let metas: Value = resp
        .json()
        .await
        .map_err(|e| RouterError::Upstream(format!("serve json from {list_url}: {e}")))?;
    let arr = metas.as_array().cloned().unwrap_or_default();
    // A 200 means the bearer is still valid. Auto-sync the most recently created
    // active key: serve returns its plaintext secret, so store it as the durable
    // Bearer credential and clear any prior deletion marker. An empty list (or a
    // list with no usable secret) means the account no longer has a key on serve —
    // mark it deleted (clear the stale secret) so the dashboard shows a degraded
    // state. Network/parse failures above already bail out, leaving prior state.
    let mut keys = st.keys.lock().unwrap();
    match serve_pick_api_key(&arr) {
        Some((name, _prefix, secret)) => {
            // Storing a fresh key clears any prior deletion marker.
            keys.oauth_set_api_key(&secret, Some(&name))
                .map_err(AppError)?;
        }
        None => {
            // Bearer still valid but no usable key returned: the prior key was
            // deleted/revoked on serve. Clear the stale secret (keep name/prefix).
            if keys.oauth_public().api_key_configured || keys.oauth_public().api_key_deleted {
                keys.oauth_mark_api_key_deleted().map_err(AppError)?;
            }
        }
    }
    Ok(Json(keys.oauth_public()))
}

// Manual paste of the serve `sk-bf-` secret is no longer supported. Serve's
// `GET /api/api-keys` returns the plaintext secret and `serve_account_sync`
// stores it automatically, so the old `POST /v1/router/serve/account/key`
// endpoint has been removed.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::UserStore;
    use aria_router_config::RouterDocument;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use serde_json::json;

    const TEST_YAML: &str = r#"
version: v0.3
providers:
  defaults: {default_model: local/general}
  models:
    - name: local/general
      provider_model_id: echo
      backend_refs: [{name: p, endpoint: 127.0.0.1:9}]
entrypoints:
  - {model_names: [ariacompute/semantic-auto], router: semantic, recipe: r}
recipes:
  - name: r
    router: semantic
    routing: {strategy: priority, decisions: []}
global:
  require_api_key: false
  allow_register: true
"#;

    /// Isolated state (temp keys/users files) with an optional seeded admin.
    fn state(with_admin: bool) -> (Arc<AppState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let users_path = dir.path().join("users.json");
        let mut doc = RouterDocument::from_yaml_str(TEST_YAML).unwrap();
        doc.global.keys_path = Some(dir.path().join("keys.json").display().to_string());
        doc.global.users_path = Some(users_path.display().to_string());
        if with_admin {
            UserStore::create_admin(&users_path, "admin", "password1").unwrap();
        }
        (Arc::new(AppState::new(doc)), dir)
    }

    fn cookie_headers(cookie: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        h
    }

    /// Session cookie for `username`, issued directly through the store.
    fn session_for(st: &Arc<AppState>, username: &str) -> String {
        let mut users = st.users.lock().unwrap();
        let id = users
            .list_public()
            .into_iter()
            .find(|u| u.username == username)
            .unwrap_or_else(|| panic!("missing user {username}"))
            .id;
        format!("aria_router_session={}", users.issue_session(&id).unwrap())
    }

    async fn json_body(res: Response) -> Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn location(res: &Response) -> String {
        res.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    /// Mock Aria Compute serve exposing the link exchange + API key list.
    async fn spawn_serve(
        exchange: (u16, &'static str, String),
        keys: (u16, &'static str, String),
    ) -> String {
        let ex_code = StatusCode::from_u16(exchange.0).unwrap();
        let ex_type = exchange.1;
        let ex_body = exchange.2.clone();
        let key_code = StatusCode::from_u16(keys.0).unwrap();
        let key_type = keys.1;
        let key_body = keys.2.clone();
        let app = axum::Router::new()
            .route(
                "/api/router-link/exchange",
                axum::routing::post(move || {
                    let body = ex_body.clone();
                    async move { (ex_code, [(header::CONTENT_TYPE, ex_type)], body) }
                }),
            )
            .route(
                "/api/api-keys",
                axum::routing::get(move || {
                    let body = key_body.clone();
                    async move { (key_code, [(header::CONTENT_TYPE, key_type)], body) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    const EXCHANGE_OK: &str = r#"{"user":{"id":"serve-1","email":"new@aria.io","name":"New User","role":"user"},"link_token":"lt-1","site":"com","expires_at":"2030-01-01T00:00:00Z"}"#;

    #[tokio::test]
    async fn require_user_optional_user_and_gate() {
        let (st, _dir) = state(true);
        // No credentials at all.
        assert!(require_user(&st, &HeaderMap::new()).is_err());
        assert!(optional_user(&st, &HeaderMap::new()).is_none());
        assert!(gate_if_users(&st, &HeaderMap::new()).is_err());
        // Unknown session token.
        let h = cookie_headers("aria_router_session=nope");
        assert!(require_user(&st, &h).is_err());
        // Valid session.
        let h = cookie_headers(&session_for(&st, "admin"));
        let me = require_user(&st, &h).unwrap();
        assert_eq!(me.username, "admin");
        assert!(optional_user(&st, &h).is_some());
        assert!(gate_if_users(&st, &h).unwrap().is_some());

        // With no local users, the dashboard gate stays open.
        let (anon, _dir2) = state(false);
        assert!(gate_if_users(&anon, &HeaderMap::new()).unwrap().is_none());
    }

    #[tokio::test]
    async fn register_status_register_and_login() {
        let (st, _dir) = state(true);
        let Json(status) = register_status(State(st.clone())).await;
        assert_eq!(status["needs_setup"], false);
        assert_eq!(status["allow_register"], true);

        let res = register(
            State(st.clone()),
            Json(AuthBody {
                username: "alice".into(),
                password: "password1".into(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().contains_key(header::SET_COOKIE));
        let body = json_body(res).await;
        assert_eq!(body["user"]["username"], "alice");
        assert!(body["token"].as_str().unwrap().len() > 10);

        // Duplicate username → 400 with the reason in the body.
        let res = register(
            State(st.clone()),
            Json(AuthBody {
                username: "ALICE".into(),
                password: "password1".into(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // Login success sets a session cookie.
        let res = login(
            State(st.clone()),
            Json(AuthBody {
                username: "alice".into(),
                password: "password1".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().contains_key(header::SET_COOKIE));
        let body = json_body(res).await;
        let token = body["token"].as_str().unwrap().to_string();

        // The issued session resolves to the same user.
        let h = cookie_headers(&format!("aria_router_session={token}"));
        let Json(me_body) = me(State(st.clone()), h.clone()).await.unwrap();
        assert_eq!(me_body["user"]["username"], "alice");

        // Logout clears the cookie and invalidates the session.
        let res = logout(State(st.clone()), h.clone()).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));
        assert!(me(State(st.clone()), h).await.is_err());

        // Bad password / unknown user are Unauthorized.
        let err = login(
            State(st.clone()),
            Json(AuthBody {
                username: "alice".into(),
                password: "wrong-password".into(),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::Unauthorized(_)), "{:?}", err.0);
    }

    #[tokio::test]
    async fn register_fail_closed_and_disabled_registration() {
        // No local users yet → 503 (run setup first).
        let (anon, _dir) = state(false);
        let res = register(
            State(anon.clone()),
            Json(AuthBody {
                username: "x".into(),
                password: "password1".into(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let Json(status) = register_status(State(anon)).await;
        assert_eq!(status["needs_setup"], true);
        assert_eq!(status["allow_register"], false);

        // allow_register=false → 401 with the reason.
        let (st, _dir2) = state(true);
        st.doc.lock().unwrap().global.allow_register = false;
        let res = register(
            State(st),
            Json(AuthBody {
                username: "x".into(),
                password: "password1".into(),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(json_body(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("registration disabled"));
    }

    #[tokio::test]
    async fn user_management_requires_admin() {
        let (st, _dir) = state(true);
        // A regular user is created through the store to keep the flow explicit.
        st.users
            .lock()
            .unwrap()
            .register("bob", "password1", true)
            .unwrap();
        let user_h = cookie_headers(&session_for(&st, "bob"));
        let admin_h = cookie_headers(&session_for(&st, "admin"));
        let bob_id = st
            .users
            .lock()
            .unwrap()
            .list_public()
            .into_iter()
            .find(|u| u.username == "bob")
            .unwrap()
            .id;

        // Non-admin cannot list or mutate users.
        let err = list_users(State(st.clone()), user_h.clone())
            .await
            .unwrap_err();
        assert!(matches!(err.0, RouterError::Unauthorized(_)), "{:?}", err.0);
        let err = set_user_disabled(
            State(st.clone()),
            user_h.clone(),
            axum::extract::Path(bob_id.clone()),
            Json(DisableBody { disabled: true }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::Unauthorized(_)), "{:?}", err.0);
        let err = set_user_email(
            State(st.clone()),
            user_h.clone(),
            axum::extract::Path(bob_id.clone()),
            Json(EmailBody { email: None }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::Unauthorized(_)), "{:?}", err.0);
        let err = set_allow_register(
            State(st.clone()),
            user_h,
            Json(AllowRegisterBody {
                allow_register: false,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::Unauthorized(_)), "{:?}", err.0);

        // Admin can list users.
        let Json(list) = list_users(State(st.clone()), admin_h.clone())
            .await
            .unwrap();
        assert_eq!(list["users"].as_array().unwrap().len(), 2);

        // Admin disables / re-enables bob, and changes emails.
        let Json(ok) = set_user_disabled(
            State(st.clone()),
            admin_h.clone(),
            axum::extract::Path(bob_id.clone()),
            Json(DisableBody { disabled: true }),
        )
        .await
        .unwrap();
        assert_eq!(ok["ok"], true);
        let Json(ok) = set_user_disabled(
            State(st.clone()),
            admin_h.clone(),
            axum::extract::Path(bob_id.clone()),
            Json(DisableBody { disabled: false }),
        )
        .await
        .unwrap();
        assert_eq!(ok["id"], bob_id);
        let Json(ok) = set_user_email(
            State(st.clone()),
            admin_h.clone(),
            axum::extract::Path(bob_id.clone()),
            Json(EmailBody {
                email: Some("Bob@Aria.IO".into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(ok["ok"], true);
        // Unknown user id bubbles up as a bad-request error.
        let err = set_user_disabled(
            State(st.clone()),
            admin_h.clone(),
            axum::extract::Path("usr_missing".to_string()),
            Json(DisableBody { disabled: true }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::InvalidParam(_)), "{:?}", err.0);

        // Admin toggles global registration.
        let Json(ok) = set_allow_register(
            State(st.clone()),
            admin_h.clone(),
            Json(AllowRegisterBody {
                allow_register: false,
            }),
        )
        .await
        .unwrap();
        assert_eq!(ok["allow_register"], false);
        assert!(!st.doc.lock().unwrap().global.allow_register);
    }

    #[tokio::test]
    async fn self_service_password_and_email() {
        let (st, _dir) = state(true);
        let h = cookie_headers(&session_for(&st, "admin"));
        let Json(ok) = change_password(
            State(st.clone()),
            h.clone(),
            Json(PasswordBody {
                password: "password2".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(ok["ok"], true);
        assert!(st.users.lock().unwrap().login("admin", "password2").is_ok());

        // Too-short password is rejected.
        let err = change_password(
            State(st.clone()),
            h.clone(),
            Json(PasswordBody {
                password: "short".into(),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::InvalidParam(_)), "{:?}", err.0);

        // Email set/clear for the current user.
        let Json(ok) = set_my_email(
            State(st.clone()),
            h.clone(),
            Json(EmailBody {
                email: Some("me@aria.io".into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(ok["ok"], true);
        let Json(ok) = set_my_email(State(st.clone()), h, Json(EmailBody { email: None }))
            .await
            .unwrap();
        assert_eq!(ok["ok"], true);

        // Unauthenticated callers are rejected.
        assert!(change_password(
            State(st),
            HeaderMap::new(),
            Json(PasswordBody {
                password: "password3".into()
            }),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn serve_account_get_requires_login_when_users_exist() {
        let (st, _dir) = state(true);
        assert!(serve_account_get(State(st.clone()), HeaderMap::new())
            .await
            .is_err());
        let h = cookie_headers(&session_for(&st, "admin"));
        let Json(acct) = serve_account_get(State(st.clone()), h).await.unwrap();
        assert!(!acct.linked);
        assert_eq!(acct.status, "not linked");
    }

    #[tokio::test]
    async fn oauth_start_builds_authorize_url() {
        let (st, _dir) = state(true);
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8899"));
        let Json(v) = oauth_start(
            State(st.clone()),
            h,
            Json(OAuthStartBody {
                site: Some("cn".into()),
            }),
        )
        .await
        .unwrap();
        let url = v["authorize_url"].as_str().unwrap();
        assert!(
            url.starts_with("https://ariacompute.cn/api/router-link/start"),
            "{url}"
        );
        assert!(!url.contains("&email="), "{url}");
        assert_eq!(v["site_url"], "https://ariacompute.cn");
        assert_eq!(
            v["callback"],
            "http://127.0.0.1:8899/v1/router/auth/oauth/callback"
        );
        assert_eq!(v["state"].as_str().unwrap().len(), 32);

        // Site falls back to the document's serve_site and Host to the default.
        let Json(v) = oauth_start(
            State(st.clone()),
            HeaderMap::new(),
            Json(OAuthStartBody {
                site: Some(" ".into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(v["site_url"], "https://ariacompute.com");
        assert_eq!(
            v["callback"],
            "http://127.0.0.1:8080/v1/router/auth/oauth/callback"
        );

        // Unknown site fails closed.
        let err = oauth_start(
            State(st),
            HeaderMap::new(),
            Json(OAuthStartBody {
                site: Some("mars".into()),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err.0, RouterError::InvalidParam(_)), "{:?}", err.0);
    }

    #[tokio::test]
    async fn oauth_callback_rejects_bad_requests() {
        let (st, _dir) = state(true);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: None,
                state: None,
                error: Some("access denied".into()),
            }),
        )
        .await;
        assert_eq!(location(&res), "/?error=access%20denied");

        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: None,
                state: Some("s".into()),
                error: None,
            }),
        )
        .await;
        assert_eq!(location(&res), "/?error=missing_code");

        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some("unknown".into()),
                error: None,
            }),
        )
        .await;
        assert!(location(&res).starts_with("/?error="), "{res:?}");
        assert!(location(&res).contains("invalid"));
    }

    #[tokio::test]
    async fn oauth_callback_links_account_and_issues_session() {
        let (st, _dir) = state(true);
        let base = spawn_serve(
            (200, "application/json", EXCHANGE_OK.to_string()),
            (
                200,
                "application/json",
                json!([{
                    "name": "auto",
                    "prefix": "sk-bf-AUTO",
                    "secret": "sk-bf-AUTOKEY1234567",
                    "created_at": "2025-01-01T00:00:00Z"
                }])
                .to_string(),
            ),
        )
        .await;
        let state_id = st.keys.lock().unwrap().begin_link_at_for_test(&base);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("code-1".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(location(&res), "/?oauth=1");
        assert!(res.headers().contains_key(header::SET_COOKIE));

        // The serve identity became a dashboard user (regular role: an admin
        // already exists and the email is not whitelisted).
        let users = st.users.lock().unwrap().list_public();
        let linked = users
            .iter()
            .find(|u| u.username == "new")
            .expect("oauth user created");
        assert_eq!(linked.role, UserRole::User);
        assert_eq!(linked.email.as_deref(), Some("new@aria.io"));
        drop(users);
        let acct = st.keys.lock().unwrap().oauth_public();
        assert!(acct.linked);
        assert_eq!(acct.site.as_deref(), Some("intl"));
        assert_eq!(acct.site_url.as_deref(), Some("https://ariacompute.com"));
        assert_eq!(acct.api_key_name.as_deref(), Some("aria-router"));

        // The detached key sync adopted serve's plaintext secret.
        let mut synced = None;
        for _ in 0..50 {
            if let Some(k) = st.keys.lock().unwrap().oauth_api_key() {
                synced = Some(k);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(synced.as_deref(), Some("sk-bf-AUTOKEY1234567"));
    }

    #[tokio::test]
    async fn oauth_callback_surfaces_exchange_errors() {
        let (st, _dir) = state(true);
        // Exchange reports an error object.
        let base = spawn_serve(
            (
                200,
                "application/json",
                json!({"error": "code expired"}).to_string(),
            ),
            (200, "application/json", "[]".into()),
        )
        .await;
        let state_id = st.keys.lock().unwrap().begin_link_at_for_test(&base);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert_eq!(location(&res), "/?error=code%20expired");

        // Non-JSON exchange bodies are reported, not swallowed.
        let base = spawn_serve(
            (200, "text/plain", "not-json".into()),
            (200, "application/json", "[]".into()),
        )
        .await;
        let state_id = st.keys.lock().unwrap().begin_link_at_for_test(&base);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert!(location(&res).starts_with("/?error="), "{res:?}");

        // Unreachable serve → connection error surfaced as a redirect.
        let state_id = st
            .keys
            .lock()
            .unwrap()
            .begin_link_at_for_test("http://127.0.0.1:1");
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert!(location(&res).starts_with("/?error="), "{res:?}");

        // A response without a usable serve user id is rejected.
        let base = spawn_serve(
            (
                200,
                "application/json",
                json!({"user": {"id": null}}).to_string(),
            ),
            (200, "application/json", "[]".into()),
        )
        .await;
        let state_id = st.keys.lock().unwrap().begin_link_at_for_test(&base);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert_eq!(location(&res), "/?error=missing_serve_id");
        assert!(!st.keys.lock().unwrap().oauth_public().linked);
    }

    #[tokio::test]
    async fn oauth_callback_switches_account_and_clears_previous_key() {
        let (st, _dir) = state(true);
        // First link: numeric serve id, no link_token, no site field.
        let base = spawn_serve(
            (
                200,
                "application/json",
                json!({"user": {"id": 42, "email": null}}).to_string(),
            ),
            (200, "application/json", "[]".into()),
        )
        .await;
        let state_id = st.keys.lock().unwrap().begin_link_at_for_test(&base);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert_eq!(location(&res), "/?oauth=1");
        let first_user = st.users.lock().unwrap().list_public()[0].clone();
        st.keys
            .lock()
            .unwrap()
            .oauth_set_api_key("sk-bf-FIRSTKEY123456", Some("first"))
            .unwrap();

        // Second link: different serve identity → previous key dropped.
        let base = spawn_serve(
            (
                200,
                "application/json",
                json!({"user": {"id": "serve-2", "email": "two@aria.io"}}).to_string(),
            ),
            (200, "application/json", "[]".into()),
        )
        .await;
        let state_id = st.keys.lock().unwrap().begin_link_at_for_test(&base);
        let res = oauth_callback(
            State(st.clone()),
            Query(LinkCallbackQ {
                code: Some("c".into()),
                state: Some(state_id),
                error: None,
            }),
        )
        .await;
        assert_eq!(location(&res), "/?oauth=1");
        let keys = st.keys.lock().unwrap();
        assert!(keys.oauth_api_key().is_none(), "stale key dropped");
        let owner = keys.oauth_owner_user_id();
        drop(keys);
        let second = st
            .users
            .lock()
            .unwrap()
            .list_public()
            .into_iter()
            .find(|u| u.username == "two")
            .expect("second serve identity created");
        assert_eq!(owner.as_deref(), Some(second.id.as_str()));
        assert_ne!(first_user.id, second.id);
        assert_eq!(st.users.lock().unwrap().list_public().len(), 3);
    }

    #[tokio::test]
    async fn serve_fetch_api_keys_reports_upstream_failures() {
        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (
                500,
                "application/json",
                json!({"error": "boom"}).to_string(),
            ),
        )
        .await;
        let err = serve_fetch_api_keys(&base, "sk-bf-X").await.unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)), "{err}");
        assert!(err.to_string().contains("status 500"), "{err}");

        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (200, "text/plain", "nope".into()),
        )
        .await;
        let err = serve_fetch_api_keys(&base, "sk-bf-X").await.unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)), "{err}");

        // Connection failures are Upstream errors too.
        let err = serve_fetch_api_keys("http://127.0.0.1:1", "sk-bf-X")
            .await
            .unwrap_err();
        assert!(matches!(err, RouterError::Upstream(_)), "{err}");

        // A non-array payload yields an empty list rather than an error.
        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (200, "application/json", "{}".into()),
        )
        .await;
        assert!(serve_fetch_api_keys(&base, "sk-bf-X")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn sync_serve_key_stores_secret_and_swallows_failures() {
        // Success: the detached-task body stores the plaintext secret.
        let (st, _dir) = state(true);
        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (
                200,
                "application/json",
                json!([{
                    "name": "linked",
                    "prefix": "sk-bf-LINK",
                    "secret": "sk-bf-LINKSECRET123456"
                }])
                .to_string(),
            ),
        )
        .await;
        sync_serve_key(st.clone(), base, "lt-link".into()).await;
        assert_eq!(
            st.keys.lock().unwrap().oauth_api_key().as_deref(),
            Some("sk-bf-LINKSECRET123456")
        );

        // Failure: an unreachable serve must not touch the stored credential.
        let (st, _dir2) = state(true);
        sync_serve_key(st.clone(), "http://127.0.0.1:1".into(), "lt-link".into()).await;
        assert!(st.keys.lock().unwrap().oauth_api_key().is_none());
    }

    #[tokio::test]
    async fn serve_account_sync_requires_link_and_credential() {
        let (st, _dir) = state(true);
        let h = cookie_headers(&session_for(&st, "admin"));
        let err = serve_account_sync(State(st.clone()), h.clone())
            .await
            .unwrap_err();
        assert!(matches!(err.0, RouterError::InvalidParam(_)), "{:?}", err.0);

        // A site-only row has neither an API key nor a link token.
        st.keys.lock().unwrap().oauth_set_site("cn").unwrap();
        let err = serve_account_sync(State(st.clone()), h.clone())
            .await
            .unwrap_err();
        assert!(
            err.0.to_string().contains("no serve credential"),
            "{:?}",
            err.0
        );

        // Non-2xx (not 401/403) surfaces the status.
        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (500, "application/json", "{}".into()),
        )
        .await;
        {
            let mut keys = st.keys.lock().unwrap();
            keys.set_site_url_for_test(&base);
            keys.oauth_set_api_key("sk-bf-SYNCKEY1234567", None)
                .unwrap();
        }
        let err = serve_account_sync(State(st.clone()), h.clone())
            .await
            .unwrap_err();
        assert!(matches!(err.0, RouterError::Upstream(_)), "{:?}", err.0);

        // 200 with a usable key replaces the stored secret and clears the marker.
        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (
                200,
                "application/json",
                json!([{
                    "name": "fresh",
                    "prefix": "sk-bf-FRESH",
                    "secret": "sk-bf-FRESHKEY123456",
                    "created_at": "2025-02-02T00:00:00Z"
                }])
                .to_string(),
            ),
        )
        .await;
        {
            let mut keys = st.keys.lock().unwrap();
            // A short-lived link token keeps sync callable after the stored key
            // was flagged as deleted on serve.
            keys.apply_exchange(crate::keys::ExchangeInput {
                site: "intl".into(),
                site_url: "https://ariacompute.com".into(),
                user: crate::serve_account::ServeUserInfo {
                    id: serde_json::Value::String("serve-1".into()),
                    email: None,
                    role: None,
                },
                link_token: Some("lt-sync".into()),
                expires_at: None,
                api_key: None,
                owner_user_id: None,
            })
            .unwrap();
            keys.oauth_mark_api_key_deleted().unwrap();
            keys.set_site_url_for_test(&base);
        }
        let Json(acct) = serve_account_sync(State(st.clone()), h.clone())
            .await
            .unwrap();
        assert!(!acct.api_key_deleted);
        assert_eq!(acct.api_key_name.as_deref(), Some("fresh"));
        assert_eq!(
            st.keys.lock().unwrap().oauth_api_key().as_deref(),
            Some("sk-bf-FRESHKEY123456")
        );

        // 200 without any usable key and a previously configured key → deleted.
        let base = spawn_serve(
            (200, "application/json", "{}".into()),
            (200, "application/json", "[]".into()),
        )
        .await;
        st.keys.lock().unwrap().set_site_url_for_test(&base);
        let Json(acct) = serve_account_sync(State(st.clone()), h.clone())
            .await
            .unwrap();
        assert!(acct.api_key_deleted);
        assert!(!acct.api_key_configured);

        // 200 with no usable key and nothing configured before → nothing to clear,
        // so no deletion marker is written.
        let (fresh, _fresh_dir) = state(true);
        let fresh_h = cookie_headers(&session_for(&fresh, "admin"));
        let empty_base = spawn_serve(
            (200, "application/json", "{}".into()),
            (200, "application/json", "[]".into()),
        )
        .await;
        {
            let mut keys = fresh.keys.lock().unwrap();
            keys.apply_exchange(crate::keys::ExchangeInput {
                site: "intl".into(),
                site_url: "https://ariacompute.com".into(),
                user: crate::serve_account::ServeUserInfo {
                    id: serde_json::Value::String("serve-fresh".into()),
                    email: None,
                    role: None,
                },
                link_token: Some("lt-fresh".into()),
                expires_at: None,
                api_key: None,
                owner_user_id: None,
            })
            .unwrap();
            keys.set_site_url_for_test(&empty_base);
        }
        let Json(acct) = serve_account_sync(State(fresh), fresh_h).await.unwrap();
        assert!(!acct.api_key_configured);
        assert!(!acct.api_key_deleted);

        // Unauthenticated dashboards cannot sync.
        assert!(serve_account_sync(State(st), HeaderMap::new())
            .await
            .is_err());
    }

    #[test]
    fn session_cookie_helpers() {
        let c = session_cookie("tok123");
        assert!(c.contains("aria_router_session=tok123"));
        assert!(c.contains("HttpOnly"));
        let clear = clear_session_cookie();
        assert!(clear.contains("Max-Age=0"));
    }

    #[test]
    fn urlencoding_encode_specials() {
        assert_eq!(
            urlencoding_encode("hello world?x=1"),
            "hello%20world%3Fx%3D1"
        );
        assert_eq!(urlencoding_encode("AZaz09-._~"), "AZaz09-._~");
    }

    #[test]
    fn serve_pick_api_key_newest_wins() {
        let list = vec![
            json!({
                "name": "old",
                "prefix": "sk-bf-old",
                "secret": "sk-bf-oldsecret",
                "created_at": "2024-01-01T00:00:00Z"
            }),
            json!({
                "name": "new",
                "prefix": "sk-bf-new",
                "secret": "sk-bf-newsecret",
                "created_at": "2025-06-01T00:00:00Z"
            }),
        ];
        let (name, prefix, secret) = serve_pick_api_key(&list).unwrap();
        assert_eq!(name, "new");
        assert_eq!(prefix, "sk-bf-new");
        assert_eq!(secret, "sk-bf-newsecret");

        assert!(serve_pick_api_key(&[]).is_none());
        assert!(serve_pick_api_key(&[json!({
            "name": "x",
            "prefix": "p",
            "secret": "",
            "created_at": "2025-01-01T00:00:00Z"
        })])
        .is_none());
    }
}
