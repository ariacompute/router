//! OpenAI data plane + management API + routing pipeline.

mod topology;
pub(crate) mod keys;
mod cost;
mod users;
mod serve_account;
mod auth_api;
mod embed;

use aria_router_agent::{request_view, task_from, BuiltinAgent, ToolRuntime};
use aria_router_algorithm::{hard_filter, select, RuntimeStats};
use aria_router_config::{resolve_keys_path, resolve_users_path, Recipe, RouterDocument};
use aria_router_core::{
    ChatRequest, RouteDecision, RouterError, RouterKind,
};
use aria_router_decision::select_decision;
use aria_router_plugin::{apply_request, extra_headers, remember_response, PluginHost, PluginOutcome};
use aria_router_provider::{forward, forward_sse_text, PoolState};
use aria_router_signal::extract;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use cost::{cost_usd, estimate_tokens, now_rfc3339, CostEvent, CostLedger};
pub use keys::{extract_bearer, validate_oauth_key, AuthIdentity, KeyStore};
use users::{UserRole, UserStore};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};

pub use users::UserStore as LocalUserStore;
pub use embed::mgmt_router_serve_dashboard;

pub struct AppState {
    pub doc: Mutex<RouterDocument>,
    pub config_path: PathBuf,
    pub pool: PoolState,
    pub plugins: PluginHost,
    pub last_route: Mutex<Option<RouteDecision>>,
    pub replay: Mutex<Vec<RouteDecision>>,
    pub fake_agents: Mutex<HashMap<String, RouteDecision>>,
    pub keys: Mutex<KeyStore>,
    pub cost: Mutex<CostLedger>,
    pub users: Mutex<UserStore>,
}

impl AppState {
    pub fn new(doc: RouterDocument) -> Self {
        Self::with_path(doc, PathBuf::new())
    }

    pub fn with_path(doc: RouterDocument, config_path: PathBuf) -> Self {
        let keys_path = doc
            .global
            .keys_path
            .as_deref()
            .map(|p| resolve_keys_path(p).unwrap_or_else(|_| PathBuf::from(p)))
            .unwrap_or_else(|| {
                resolve_keys_path("~/.ariacompute/router-keys.json")
                    .unwrap_or_else(|_| PathBuf::from("router-keys.json"))
            });
        let users_path = doc
            .global
            .users_path
            .as_deref()
            .map(|p| resolve_users_path(p).unwrap_or_else(|_| PathBuf::from(p)))
            .unwrap_or_else(|| {
                resolve_users_path("~/.ariacompute/router-users.json")
                    .unwrap_or_else(|_| PathBuf::from("router-users.json"))
            });
        let keys = KeyStore::load(&keys_path).unwrap_or_else(|_| KeyStore::empty(keys_path));
        let users = UserStore::load(&users_path).unwrap_or_else(|_| UserStore::empty(users_path));
        Self {
            doc: Mutex::new(doc),
            config_path,
            pool: PoolState::default(),
            plugins: PluginHost::default(),
            last_route: Mutex::new(None),
            replay: Mutex::new(Vec::new()),
            fake_agents: Mutex::new(HashMap::new()),
            keys: Mutex::new(keys),
            cost: Mutex::new(CostLedger::default()),
            users: Mutex::new(users),
        }
    }

    pub fn set_fake_agent(&self, name: &str, d: RouteDecision) {
        self.fake_agents.lock().unwrap().insert(name.to_string(), d);
    }

    pub fn require_api_key(&self) -> bool {
        self.doc.lock().unwrap().global.require_api_key
    }
}

pub fn data_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_data))
        .with_state(state)
}

pub fn mgmt_router(state: Arc<AppState>) -> Router {
    mgmt_api_router(state)
}

/// Management JSON API plus optional SPA fallback from `static_dir`.
pub fn mgmt_router_with_dashboard(state: Arc<AppState>, static_dir: impl AsRef<FsPath>) -> Router {
    let dir = static_dir.as_ref().to_path_buf();
    let index = dir.join("index.html");
    let files = tower_http::services::ServeDir::new(dir)
        .append_index_html_on_directories(true)
        .fallback(tower_http::services::ServeFile::new(index));
    mgmt_api_router(state).fallback_service(files)
}

/// Resolve `dashboard/dist` next to CWD, the binary, or `ARIA_ROUTER_DASHBOARD`.
pub fn resolve_dashboard_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ARIA_ROUTER_DASHBOARD") {
        let p = PathBuf::from(p);
        if p.join("index.html").is_file() {
            return Some(p);
        }
    }
    let mut cands = vec![PathBuf::from("dashboard/dist")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            cands.push(parent.join("dashboard/dist"));
            cands.push(parent.join("../dashboard/dist"));
            cands.push(parent.join("../../dashboard/dist"));
        }
    }
    cands.into_iter().find(|p| p.join("index.html").is_file())
}

fn mgmt_api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/router/version", get(version_ep))
        .route("/v1/router/auth/register-status", get(auth_api::register_status))
        .route("/v1/router/auth/register", post(auth_api::register))
        .route("/v1/router/auth/login", post(auth_api::login))
        .route("/v1/router/auth/logout", post(auth_api::logout))
        .route("/v1/router/auth/me", get(auth_api::me))
        .route("/v1/router/auth/email", put(auth_api::set_my_email))
        .route("/v1/router/auth/password", post(auth_api::change_password))
        .route("/v1/router/users", get(auth_api::list_users))
        .route("/v1/router/users/{id}/disabled", put(auth_api::set_user_disabled))
        .route("/v1/router/users/{id}/email", put(auth_api::set_user_email))
        .route(
            "/v1/router/settings/allow_register",
            put(auth_api::set_allow_register),
        )
        .route("/v1/router/auth/oauth/start", post(auth_api::oauth_start))
        .route("/v1/router/auth/oauth/callback", get(auth_api::oauth_callback))
        .route("/v1/router/serve/account", get(auth_api::serve_account_get))
        .route(
            "/v1/router/serve/account/sync",
            post(auth_api::serve_account_sync),
        )
        .route("/v1/router/validate", post(validate_ep))
        .route("/v1/router/replay", get(replay_ep))
        .route("/v1/router/providers", put(upsert_provider).get(list_providers))
        .route("/v1/router/config", get(get_config).put(put_config))
        .route("/v1/router/overview", get(overview_ep))
        .route("/v1/router/topology", get(topology_ep))
        .route("/v1/router/chat", post(chat_mgmt))
        .route("/v1/router/cost", get(cost_ep))
        .route("/v1/router/keys", get(list_keys).post(create_key))
        .route("/v1/router/keys/{id}", delete(revoke_key))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

/// Build metadata (version + short commit hash) for the dashboard footer.
/// Public (no auth) — mirrors harness/ariaterm's version@commit scheme and
/// leaks nothing sensitive, avoiding an extra authenticated request on load.
async fn version_ep() -> Json<Value> {
    Json(json!({
        "version": env!("ARIA_ROUTER_VERSION"),
        "commit": env!("ARIA_ROUTER_COMMIT"),
    }))
}

async fn validate_ep(State(st): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    validate_doc(&st).map_err(AppError)?;
    Ok(Json(json!({"ok": true})))
}

fn validate_doc(st: &AppState) -> Result<(), RouterError> {
    st.doc.lock().unwrap().validate()
}

#[derive(Deserialize)]
struct ReplayQ {
    n: Option<usize>,
}

async fn replay_ep(
    State(st): State<Arc<AppState>>,
    Query(q): Query<ReplayQ>,
) -> Json<Value> {
    let n = q.n.unwrap_or(20).min(100);
    Json(json!({"items": replay_items(&st, n)}))
}

fn replay_items(st: &AppState, n: usize) -> Vec<RouteDecision> {
    st.replay
        .lock()
        .unwrap()
        .iter()
        .rev()
        .take(n)
        .cloned()
        .collect()
}

#[derive(Deserialize)]
pub struct ProviderUpsert {
    pub name: String,
    pub endpoint: String,
    #[serde(default)]
    pub provider_model_id: String,
    #[serde(default)]
    pub locality: Option<String>,
}

async fn upsert_provider(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ProviderUpsert>,
) -> Result<Json<Value>, AppError> {
    auth_provider_upsert(&st, &headers).map_err(AppError)?;
    apply_upsert(&st, body).map_err(AppError)
}

fn auth_provider_upsert(st: &AppState, headers: &HeaderMap) -> Result<(), RouterError> {
    let require = st.require_api_key();
    match extract_bearer(headers) {
        Some(secret) => {
            let mut keys = st.keys.lock().unwrap();
            keys.resolve_bearer(&secret)?;
            Ok(())
        }
        None if require => Err(RouterError::Unauthorized("api key required".into())),
        None => Ok(()),
    }
}

fn apply_upsert(st: &AppState, body: ProviderUpsert) -> Result<Json<Value>, RouterError> {
    let mut doc = st.doc.lock().unwrap();
    if let Some(existing) = doc.providers.models.iter_mut().find(|m| m.name == body.name) {
        existing.backend_refs = vec![aria_router_config::BackendRef {
            name: "engine".into(),
            endpoint: body.endpoint.clone(),
            base_url: String::new(),
            protocol: "http".into(),
            weight: 100,
            api_key: None,
            api_key_env: None,
        }];
        if !body.provider_model_id.is_empty() {
            existing.provider_model_id = body.provider_model_id;
        }
    } else {
        doc.providers.models.push(aria_router_config::ProviderModel {
            name: body.name.clone(),
            provider_model_id: if body.provider_model_id.is_empty() {
                body.name.clone()
            } else {
                body.provider_model_id
            },
            locality: body.locality.unwrap_or_else(|| "local".into()),
            modality: "text".into(),
            capabilities: vec!["chat".into()],
            backend_refs: vec![aria_router_config::BackendRef {
                name: "engine".into(),
                endpoint: body.endpoint,
                base_url: String::new(),
                protocol: "http".into(),
                weight: 100,
                api_key: None,
                api_key_env: None,
            }],
            pricing: None,
        });
    }
    Ok(Json(json!({"ok": true, "name": body.name})))
}

async fn list_providers(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(providers_json(&st))
}

fn providers_json(st: &AppState) -> Value {
    let doc = st.doc.lock().unwrap();
    let lat = st.pool.latency_map();
    let fails = st.pool.failures_map();
    let models: Vec<Value> = doc
        .providers
        .models
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "provider_model_id": m.provider_model_id,
                "locality": m.locality,
                "modality": m.modality,
                "backend_refs": m.backend_refs,
                "latency_ms": lat.get(&m.name),
                "failures": fails.get(&m.name).copied().unwrap_or(0),
            })
        })
        .collect();
    json!({"models": models})
}

async fn get_config(State(st): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let doc = snapshot_doc(&st);
    let yaml = doc.to_yaml().map_err(AppError)?;
    Ok(Json(json!({"document": doc, "yaml": yaml})))
}

async fn put_config(
    State(st): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppError> {
    let raw = String::from_utf8(body.to_vec())
        .map_err(|e| AppError(RouterError::InvalidParam(e.to_string())))?;
    let doc = parse_config_body(&raw).map_err(AppError)?;
    replace_config(&st, doc, &raw).map_err(AppError)?;
    Ok(Json(json!({"ok": true})))
}

fn parse_config_body(raw: &str) -> Result<RouterDocument, RouterError> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') {
        RouterDocument::from_json_str(raw)
    } else {
        RouterDocument::from_yaml_str(raw)
    }
}

fn replace_config(st: &AppState, doc: RouterDocument, raw: &str) -> Result<(), RouterError> {
    if !st.config_path.as_os_str().is_empty() {
        std::fs::write(&st.config_path, raw).map_err(|e| {
            RouterError::Io(format!("write {}: {e}", st.config_path.display()))
        })?;
    }
    let keys_path = doc
        .global
        .keys_path
        .as_deref()
        .map(resolve_keys_path)
        .transpose()?
        .unwrap_or_else(|| {
            resolve_keys_path("~/.ariacompute/router-keys.json")
                .unwrap_or_else(|_| PathBuf::from("router-keys.json"))
        });
    let keys = KeyStore::load(&keys_path).unwrap_or_else(|_| KeyStore::empty(keys_path));
    *st.keys.lock().unwrap() = keys;
    *st.doc.lock().unwrap() = doc;
    Ok(())
}

async fn overview_ep(State(st): State<Arc<AppState>>) -> Json<Value> {
    let doc = snapshot_doc(&st);
    let (active, revoked) = st.keys.lock().unwrap().counts();
    let (admin_n, user_n) = st.users.lock().unwrap().counts();
    let serve = st.keys.lock().unwrap().oauth_public();
    let cost = st.cost.lock().unwrap().summary();
    Json(json!({
        "status": "ok",
        "entrypoints": doc.entrypoints.len(),
        "recipes": doc.recipes.len(),
        "providers": doc.providers.models.len(),
        "last_route": st.last_route.lock().unwrap().clone(),
        "cost": cost,
        "api_keys": { "active": active, "revoked": revoked },
        "local_users": { "admin": admin_n, "user": user_n },
        "serve_account": serve,
        "allow_register": doc.global.allow_register,
    }))
}

#[derive(Deserialize)]
struct CostQ {
    n: Option<usize>,
}

async fn cost_ep(State(st): State<Arc<AppState>>, Query(q): Query<CostQ>) -> Json<Value> {
    let n = q.n.unwrap_or(20);
    Json(st.cost.lock().unwrap().report(n))
}

async fn list_keys(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let session = auth_api::gate_if_users(&st, &headers).map_err(AppError)?;
    let keys = st.keys.lock().unwrap();
    let list = match session {
        Some(u) => keys.list_for_owner(&u.id, matches!(u.role, UserRole::Admin)),
        None => keys.list_public(),
    };
    Ok(Json(json!({"keys": list})))
}

#[derive(Deserialize)]
struct CreateKeyBody {
    name: String,
}

async fn create_key(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateKeyBody>,
) -> Result<Json<Value>, AppError> {
    let session = auth_api::gate_if_users(&st, &headers).map_err(AppError)?;
    let owner = session.map(|u| u.id);
    let created = st
        .keys
        .lock()
        .unwrap()
        .create_for(&body.name, owner)
        .map_err(AppError)?;
    Ok(Json(serde_json::to_value(created).unwrap()))
}

async fn revoke_key(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let session = auth_api::gate_if_users(&st, &headers).map_err(AppError)?;
    if let Some(u) = session {
        if !matches!(u.role, UserRole::Admin) {
            let owner = st.keys.lock().unwrap().owner_of(&id);
            match owner {
                Some(Some(oid)) if oid == u.id => {}
                Some(None) => {}
                _ => {
                    return Err(AppError(RouterError::Unauthorized(
                        "cannot revoke another user's key".into(),
                    )));
                }
            }
        }
    }
    st.keys.lock().unwrap().revoke(&id).map_err(AppError)?;
    Ok(Json(json!({"ok": true, "id": id})))
}

async fn topology_ep(State(st): State<Arc<AppState>>) -> Json<Value> {
    let doc = snapshot_doc(&st);
    Json(topology::topology_graph(&doc))
}

async fn list_models(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(list_models_json(&st))
}

fn list_models_json(st: &AppState) -> Value {
    let doc = st.doc.lock().unwrap();
    let mut data = vec![];
    for ep in &doc.entrypoints {
        for n in &ep.model_names {
            data.push(json!({"id": n, "object": "model", "owned_by": "aria-router"}));
        }
    }
    for m in &doc.providers.models {
        data.push(json!({"id": m.name, "object": "model", "owned_by": "provider"}));
    }
    json!({"object": "list", "data": data})
}

async fn chat_data(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    chat_inner(st, headers, body, false).await
}

async fn chat_mgmt(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    chat_inner(st, headers, body, true).await
}

async fn chat_inner(
    st: Arc<AppState>,
    headers: HeaderMap,
    body: Value,
    playground: bool,
) -> Response {
    let auth = if playground {
        let sess_user = auth_api::optional_user(&st, &headers);
        if let Some(u) = sess_user {
            Ok(ChatAuth {
                user: u.username,
                key_id: None,
                key_name: None,
                identity: "local_user".into(),
                serve_user_id: None,
                serve_email: None,
                serve_site: None,
            })
        } else {
            Ok(ChatAuth {
                user: "playground".into(),
                key_id: None,
                key_name: None,
                identity: "playground".into(),
                serve_user_id: None,
                serve_email: None,
                serve_site: None,
            })
        }
    } else {
        resolve_chat_auth(&st, &headers, &body)
    };
    let auth = match auth {
        Ok(v) => v,
        Err(e) => return AppError(e).into_response(),
    };
    let req: ChatRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return AppError(RouterError::InvalidParam(e.to_string())).into_response();
        }
    };
    let session = session_id(&headers, &req);
    let metadata = metadata_from_headers(&headers);
    let want_stream = req.stream;
    let entrypoint = req.model.clone();
    match route_and_forward(
        st,
        req,
        want_stream,
        metadata,
        CostCtx {
            user: auth.user,
            key_id: auth.key_id,
            key_name: auth.key_name,
            identity: auth.identity,
            serve_user_id: auth.serve_user_id,
            serve_email: auth.serve_email,
            serve_site: auth.serve_site,
            session,
            entrypoint,
        },
    )
    .await
    {
        Ok(r) => r,
        Err(e) => AppError(e).into_response(),
    }
}

struct ChatAuth {
    user: String,
    key_id: Option<String>,
    key_name: Option<String>,
    identity: String,
    serve_user_id: Option<String>,
    serve_email: Option<String>,
    serve_site: Option<String>,
}

struct CostCtx {
    user: String,
    key_id: Option<String>,
    key_name: Option<String>,
    identity: String,
    serve_user_id: Option<String>,
    serve_email: Option<String>,
    serve_site: Option<String>,
    session: String,
    entrypoint: String,
}

struct CostUsage<'a> {
    turns_in_request: u32,
    upstream_requests: u32,
    prompt_tokens: u64,
    completion_tokens: u64,
    tokens_source: &'a str,
}

fn resolve_chat_auth(
    st: &AppState,
    headers: &HeaderMap,
    body: &Value,
) -> Result<ChatAuth, RouterError> {
    let require = st.require_api_key();
    if let Some(secret) = extract_bearer(headers) {
        let mut keys = st.keys.lock().unwrap();
        let identity = keys.resolve_bearer(&secret)?;
        drop(keys);
        return match identity {
            AuthIdentity::Oauth {
                id,
                name,
                email,
                site,
                user_id,
            } => {
                let email = email.unwrap_or_else(|| id.clone());
                Ok(ChatAuth {
                    user: email.clone(),
                    key_id: Some(id),
                    key_name: name,
                    identity: "serve".into(),
                    serve_user_id: user_id,
                    serve_email: Some(email),
                    serve_site: site,
                })
            }
            AuthIdentity::Local {
                id,
                name,
                owner_user_id,
            } => {
                let (user, identity) = if let Some(oid) = owner_user_id {
                    if let Some(u) = st.users.lock().unwrap().get(&oid) {
                        (u.username.clone(), "local_user".into())
                    } else {
                        (name.clone(), "local".into())
                    }
                } else {
                    (name.clone(), "local".into())
                };
                Ok(ChatAuth {
                    user,
                    key_id: Some(id),
                    key_name: Some(name),
                    identity,
                    serve_user_id: None,
                    serve_email: None,
                    serve_site: None,
                })
            }
        };
    }
    if require {
        return Err(RouterError::Unauthorized("api key required".into()));
    }
    let user = body
        .get("user")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .get("x-aria-user")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "anonymous".into());
    Ok(ChatAuth {
        user,
        key_id: None,
        key_name: None,
        identity: "anonymous".into(),
        serve_user_id: None,
        serve_email: None,
        serve_site: None,
    })
}

fn session_id(headers: &HeaderMap, req: &ChatRequest) -> String {
    if let Some(v) = headers.get("x-aria-session").and_then(|v| v.to_str().ok()) {
        if !v.is_empty() {
            return v.to_string();
        }
    }
    if let Some(v) = req.extra.get("session").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            return v.to_string();
        }
    }
    format!("req_{}", now_rfc3339())
}

async fn route_and_forward(
    st: Arc<AppState>,
    req: ChatRequest,
    want_stream: bool,
    metadata: HashMap<String, String>,
    ctx: CostCtx,
) -> Result<Response, RouterError> {
    let turns_in_request = req
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .count() as u32;
    let prompt_est = estimate_tokens(&req.prompt_text());
    let (decision, mut fwd, extra, hdrs) = route_request(&st, req, &metadata).await?;
    record(&st, decision.clone());
    match extra {
        Some(fast) => {
            let completion = estimate_tokens(&fast.to_string());
            record_cost(
                &st,
                &ctx,
                &decision,
                CostUsage {
                    turns_in_request,
                    upstream_requests: 0,
                    prompt_tokens: prompt_est,
                    completion_tokens: completion,
                    tokens_source: "estimate",
                },
            );
            let mut res = Json(fast).into_response();
            attach_route_headers(res.headers_mut(), &decision);
            Ok(res)
        }
        None => {
            let doc = snapshot_doc(&st);
            if want_stream {
                fwd.stream = true;
                let text = forward_sse_text(&doc, &decision.model, &fwd, &hdrs, &st.pool).await?;
                let (pt, ct, src) = parse_sse_usage(&text, prompt_est);
                record_cost(
                    &st,
                    &ctx,
                    &decision,
                    CostUsage {
                        turns_in_request,
                        upstream_requests: 1,
                        prompt_tokens: pt,
                        completion_tokens: ct,
                        tokens_source: src,
                    },
                );
                let mut res = Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(text))
                    .unwrap();
                attach_route_headers(res.headers_mut(), &decision);
                Ok(res)
            } else {
                let body = forward(&doc, &decision.model, &fwd, &hdrs, &st.pool).await?;
                remember_response(&st.plugins, &fwd, &body);
                let (pt, ct, src) = parse_json_usage(&body, prompt_est);
                record_cost(
                    &st,
                    &ctx,
                    &decision,
                    CostUsage {
                        turns_in_request,
                        upstream_requests: 1,
                        prompt_tokens: pt,
                        completion_tokens: ct,
                        tokens_source: src,
                    },
                );
                let mut res = Json(body).into_response();
                attach_route_headers(res.headers_mut(), &decision);
                Ok(res)
            }
        }
    }
}

fn parse_json_usage(body: &Value, prompt_est: u64) -> (u64, u64, &'static str) {
    if let Some(u) = body.get("usage") {
        let pt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(prompt_est);
        let ct = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        return (pt, ct, "usage");
    }
    let ct = body
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(estimate_tokens)
        .unwrap_or(0);
    (prompt_est, ct, "estimate")
}

fn parse_sse_usage(text: &str, prompt_est: u64) -> (u64, u64, &'static str) {
    for line in text.lines().rev() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(data) {
            if let Some(u) = v.get("usage") {
                let pt = u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(prompt_est);
                let ct = u.get("completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                return (pt, ct, "usage");
            }
        }
    }
    (prompt_est, estimate_tokens(text) / 4, "estimate")
}

fn record_cost(st: &AppState, ctx: &CostCtx, decision: &RouteDecision, usage: CostUsage<'_>) {
    let doc = snapshot_doc(st);
    let (in_p, out_p, priced) = match doc.provider(&decision.model).and_then(|p| p.pricing.as_ref()) {
        Some(p) if p.input_per_mtok > 0.0 || p.output_per_mtok > 0.0 => {
            (p.input_per_mtok, p.output_per_mtok, true)
        }
        _ => (0.0, 0.0, false),
    };
    let cost = cost_usd(usage.prompt_tokens, usage.completion_tokens, in_p, out_p);
    let ev = CostEvent {
        ts: now_rfc3339(),
        user: ctx.user.clone(),
        key_id: ctx.key_id.clone(),
        key_name: ctx.key_name.clone(),
        identity: ctx.identity.clone(),
        serve_user_id: ctx.serve_user_id.clone(),
        serve_email: ctx.serve_email.clone(),
        serve_site: ctx.serve_site.clone(),
        session: ctx.session.clone(),
        entrypoint: ctx.entrypoint.clone(),
        layer: decision.layer.clone(),
        decision: decision.decision.clone(),
        model: decision.model.clone(),
        bypass: decision.bypass,
        turns_in_request: usage.turns_in_request.max(1),
        upstream_requests: usage.upstream_requests,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        input_per_mtok: in_p,
        output_per_mtok: out_p,
        cost_usd: cost,
        tokens_source: usage.tokens_source.into(),
        priced,
    };
    st.cost.lock().unwrap().record(ev);
}

fn attach_route_headers(h: &mut HeaderMap, d: &RouteDecision) {
    let _ = h.insert("x-aria-router-layer", d.layer.parse().unwrap_or_else(|_| "none".parse().unwrap()));
    let _ = h.insert(
        "x-aria-router-decision",
        d.decision.parse().unwrap_or_else(|_| "none".parse().unwrap()),
    );
    if let Ok(v) = d.model.parse() {
        h.insert("x-aria-router-model", v);
    }
}

fn record(st: &AppState, d: RouteDecision) {
    *st.last_route.lock().unwrap() = Some(d.clone());
    let mut r = st.replay.lock().unwrap();
    r.push(d);
    if r.len() > 256 {
        r.remove(0);
    }
}

fn snapshot_doc(st: &AppState) -> RouterDocument {
    st.doc.lock().unwrap().clone()
}

pub fn last_route_json(st: &AppState) -> Value {
    match st.last_route.lock().unwrap().clone() {
        Some(d) => serde_json::to_value(d).unwrap_or(json!({})),
        None => json!({}),
    }
}

pub async fn route_request(
    st: &AppState,
    req: ChatRequest,
    metadata: &HashMap<String, String>,
) -> Result<(RouteDecision, ChatRequest, Option<Value>, Vec<(String, String)>), RouterError> {
    let doc = snapshot_doc(st);
    if doc.is_concrete_model(&req.model) {
        let d = RouteDecision::bypass(&req.model);
        return Ok((d, req, None, vec![]));
    }
    let ep = doc.entrypoint_for(&req.model).ok_or_else(|| {
        RouterError::FailClosed(format!("unknown model {}", req.model))
    })?;
    let recipe = doc.recipe(&ep.recipe)?.clone();
    if recipe.router != ep.router {
        return Err(RouterError::Config("entrypoint/recipe router mismatch".into()));
    }
    match ep.router {
        RouterKind::Semantic => route_semantic(st, &doc, &recipe, req, metadata).await,
        RouterKind::Agent => route_agent(st, &doc, &recipe, req, metadata).await,
    }
}

async fn route_semantic(
    st: &AppState,
    doc: &RouterDocument,
    recipe: &Recipe,
    req: ChatRequest,
    metadata: &HashMap<String, String>,
) -> Result<(RouteDecision, ChatRequest, Option<Value>, Vec<(String, String)>), RouterError> {
    let learned = doc.learned_signal_referenced(recipe);
    if !learned.is_empty() {
        return Err(RouterError::Unsupported(format!(
            "learned signal {} requires feature ml / weights",
            learned[0]
        )));
    }
    let routing = recipe
        .routing
        .as_ref()
        .ok_or_else(|| RouterError::Config("missing routing".into()))?;
    let signals = extract(doc, recipe, &req, metadata)?;
    let _proj = aria_router_decision::project(&routing.projections, &signals)?;
    let decision_cfg = select_decision(recipe, &signals, &routing.strategy)?;
    let (model_names, algo, plugins, dname, loc) = if let Some(d) = decision_cfg {
        (
            d.model_refs.iter().map(|m| m.model.clone()).collect::<Vec<_>>(),
            d.algorithm.clone(),
            d.plugins.clone(),
            d.name.clone(),
            d.locality.clone(),
        )
    } else {
        let def = doc.providers.defaults.default_model.clone();
        if def.is_empty() {
            return Err(RouterError::FailClosed("no matching decision and no default_model".into()));
        }
        (vec![def], Some("static".into()), vec![], "default".into(), None)
    };
    let eligible = hard_filter(doc, &model_names, loc.as_deref(), Some("text"));
    if eligible.is_empty() {
        return Err(RouterError::FailClosed("no eligible models after hard constraints".into()));
    }
    let dummy = aria_router_config::DecisionCfg {
        name: dname.clone(),
        description: None,
        priority: 0,
        rules: Default::default(),
        model_refs: eligible
            .iter()
            .map(|e| aria_router_config::ModelRef { model: e.name.clone() })
            .collect(),
        algorithm: algo.clone(),
        plugins: plugins.clone(),
        locality: loc,
    };
    let mut cost_map = HashMap::new();
    for m in &doc.providers.models {
        cost_map.insert(m.name.clone(), m.ranking_cost());
    }
    let stats = RuntimeStats {
        latency_ms: st.pool.latency_map(),
        cost: cost_map,
        ..Default::default()
    };
    let model = select(doc, &dummy, &eligible, &stats)?;
    let hdrs = extra_headers(&plugins);
    match apply_request(&st.plugins, &plugins, req)? {
        PluginOutcome::FastResponse(v) => Ok((
            RouteDecision {
                model: model.clone(),
                algorithm: algo,
                reason: format!("semantic:{dname}"),
                confidence: 1.0,
                layer: "semantic".into(),
                decision: dname,
                bypass: false,
            },
            ChatRequest {
                model: model.clone(),
                messages: vec![],
                stream: false,
                max_tokens: None,
                temperature: None,
                extra: Default::default(),
            },
            Some(v),
            hdrs,
        )),
        PluginOutcome::Continue(fwd) => Ok((
            RouteDecision {
                model: model.clone(),
                algorithm: algo,
                reason: format!("semantic:{dname}"),
                confidence: 1.0,
                layer: "semantic".into(),
                decision: dname,
                bypass: false,
            },
            {
                let mut f = fwd;
                f.model = model;
                f
            },
            None,
            hdrs,
        )),
    }
}

async fn route_agent(
    st: &AppState,
    doc: &RouterDocument,
    recipe: &Recipe,
    req: ChatRequest,
    _metadata: &HashMap<String, String>,
) -> Result<(RouteDecision, ChatRequest, Option<Value>, Vec<(String, String)>), RouterError> {
    let agent = recipe
        .agent
        .as_ref()
        .ok_or_else(|| RouterError::Config("missing agent".into()))?;
    let all_names: Vec<String> = doc.providers.models.iter().map(|m| m.name.clone()).collect();
    let eligible = hard_filter(doc, &all_names, Some("local"), Some("text"));
    if eligible.is_empty() {
        return Err(RouterError::FailClosed("no eligible models after hard constraints".into()));
    }
    let task = task_from(&req, eligible.clone(), agent);
    let tools = ToolRuntime {
        latency_ms: st.pool.latency_map(),
        failures: st.pool.failures_map(),
        request_view: request_view(&req),
    };
    let canned = st.fake_agents.lock().unwrap().get("builtin").cloned();
    let builtin = BuiltinAgent {
        endpoint: agent.endpoint.clone(),
        model: agent.model.clone().unwrap_or_else(|| "router-llm".into()),
        canned,
    };
    let mut decision = builtin.route(task, &tools).await?;
    decision.layer = "agent".into();
    if let Some(fb) = &agent.fallback {
        if !eligible.iter().any(|e| e.name == decision.model) {
            decision.model = fb.clone();
            decision.reason = format!("fallback:{fb}");
        }
    }
    if !eligible.iter().any(|e| e.name == decision.model) && doc.provider(&decision.model).is_none()
    {
        return Err(RouterError::FailClosed(format!(
            "agent model {} not eligible",
            decision.model
        )));
    }
    let mut fwd = req;
    fwd.model = decision.model.clone();
    Ok((decision, fwd, None, vec![]))
}

fn metadata_from_headers(h: &HeaderMap) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Some(v) = h.get("x-aria-role").and_then(|v| v.to_str().ok()) {
        m.insert("role".into(), v.to_string());
    }
    if let Some(v) = h.get("x-aria-metadata").and_then(|v| v.to_str().ok()) {
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(v) {
            m.extend(map);
        }
    }
    m
}

pub struct AppError(pub RouterError);

impl From<RouterError> for AppError {
    fn from(e: RouterError) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            RouterError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            RouterError::FailClosed(_) => StatusCode::FORBIDDEN,
            RouterError::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
            RouterError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            RouterError::Config(_) | RouterError::InvalidParam(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::BAD_GATEWAY,
        };
        let body = json!({"error": {"message": self.0.to_string(), "type": format!("{:?}", self.0)}});
        (status, Json(body)).into_response()
    }
}

pub fn load_keys_for_status(path: &FsPath) -> Result<(usize, usize), RouterError> {
    Ok(KeyStore::load(path)?.counts())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::ExchangeInput;
    use crate::serve_account::ServeUserInfo;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Avoid loading the developer's real `~/.ariacompute/router-{keys,users}.json`.
    fn isolated_state(mut doc: RouterDocument) -> (Arc<AppState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        doc.global.keys_path = Some(dir.path().join("router-keys.json").display().to_string());
        doc.global.users_path = Some(dir.path().join("router-users.json").display().to_string());
        (Arc::new(AppState::new(doc)), dir)
    }

    fn isolated_state_with_path(
        mut doc: RouterDocument,
        config_path: PathBuf,
    ) -> (Arc<AppState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        doc.global.keys_path = Some(dir.path().join("router-keys.json").display().to_string());
        doc.global.users_path = Some(dir.path().join("router-users.json").display().to_string());
        (Arc::new(AppState::with_path(doc, config_path)), dir)
    }

    fn tiny_yaml(backend: &str) -> String {
        format!(
            r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/general
  models:
    - name: local/general
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: {backend}
entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: mom
  - model_names: [ariacompute/agent-auto]
    router: agent
    recipe: agent-default
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      signals:
        keywords:
          - name: needs_explain
            operator: OR
            keywords: ["explain", "walk me through"]
      decisions:
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
  - name: agent-default
    router: agent
    agent:
      max_turns: 3
      timeout_ms: 5000
      fallback: local/general
global:
  require_api_key: false
"#
        )
    }

    async fn mock_upstream() -> String {
        use axum::routing::post;
        async fn echo(Json(v): Json<Value>) -> Json<Value> {
            let model = v.get("model").cloned().unwrap_or(json!("echo"));
            Json(json!({
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "ok"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 3,
                    "total_tokens": 15
                }
            }))
        }
        let app = Router::new().route("/v1/chat/completions", post(echo));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn semantic_keyword_hit() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st);
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [{"role":"user","content":"please explain rust"}]
        });
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.headers().get("x-aria-router-layer").unwrap(),
            "semantic"
        );
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "ok");
    }

    #[tokio::test]
    async fn concrete_bypass() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st);
        let body = json!({
            "model": "local/general",
            "messages": [{"role":"user","content":"hi"}]
        });
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.headers().get("x-aria-router-layer").unwrap(),
            "bypass"
        );
    }

    #[tokio::test]
    async fn agent_builtin_and_isolation() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        st.set_fake_agent(
            "builtin",
            RouteDecision {
                model: "local/general".into(),
                algorithm: Some("static".into()),
                reason: "fake".into(),
                confidence: 0.9,
                layer: "agent".into(),
                decision: "agent".into(),
                bypass: false,
            },
        );
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/agent-auto",
            "messages": [{"role":"user","content":"hi"}]
        });
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers().get("x-aria-router-layer").unwrap(), "agent");

        let app2 = data_router(st);
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [{"role":"user","content":"please explain"}]
        });
        let res = app2
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.headers().get("x-aria-router-layer").unwrap(), "semantic");
    }

    #[tokio::test]
    async fn agent_overreach_fail_closed() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        st.set_fake_agent(
            "builtin",
            RouteDecision {
                model: "not-a-model".into(),
                algorithm: None,
                reason: "bad".into(),
                confidence: 1.0,
                layer: "agent".into(),
                decision: "agent".into(),
                bypass: false,
            },
        );
        let app = data_router(st);
        let body = json!({
            "model": "ariacompute/agent-auto",
            "messages": [{"role":"user","content":"hi"}]
        });
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn fail_closed_unknown() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st);
        let body = json!({
            "model": "nope",
            "messages": [{"role":"user","content":"hi"}]
        });
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn sse_and_upsert() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "model": "local/general",
                            "stream": true,
                            "messages": [{"role":"user","content":"hi"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.headers().get("content-type").unwrap(),
            "text/event-stream"
        );

        let admin = mgmt_router(st);
        let res = admin
            .oneshot(
                Request::put("/v1/router/providers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "engine/local",
                            "endpoint": backend,
                            "provider_model_id": "echo"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }

    async fn oneshot_json(app: Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let v = if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&bytes).unwrap_or(json!({"raw": String::from_utf8_lossy(&bytes)}))
        };
        (status, v)
    }

    #[tokio::test]
    async fn put_config_rejects_unknown_block() {
        let backend = mock_upstream().await;
        let yaml = tiny_yaml(&backend);
        let path = std::env::temp_dir().join(format!("aria-router-put-bad-{}.yml", std::process::id()));
        std::fs::write(&path, &yaml).unwrap();
        let doc = RouterDocument::from_yaml_str(&yaml).unwrap();
        let (st, _dir) = isolated_state_with_path(doc, path.clone());
        let before = snapshot_doc(&st);
        let admin = mgmt_router(st.clone());
        let (status, _) = oneshot_json(
            admin,
            Request::put("/v1/router/config")
                .header("content-type", "application/yaml")
                .body(Body::from(format!("{yaml}\nunknown_block: true\n")))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(snapshot_doc(&st).entrypoints.len(), before.entrypoints.len());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn put_config_reloads_tiny_yaml() {
        let backend = mock_upstream().await;
        let yaml = tiny_yaml(&backend);
        let path = std::env::temp_dir().join(format!("aria-router-put-ok-{}.yml", std::process::id()));
        std::fs::write(&path, &yaml).unwrap();
        let doc = RouterDocument::from_yaml_str(&yaml).unwrap();
        let (st, _dir) = isolated_state_with_path(doc, path.clone());
        let admin = mgmt_router(st.clone());
        let replaced = yaml.replace("ariacompute/semantic-auto", "ariacompute/semantic-renamed");
        let (status, body) = oneshot_json(
            admin,
            Request::put("/v1/router/config")
                .header("content-type", "application/yaml")
                .body(Body::from(replaced))
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        let got = oneshot_json(
            mgmt_router(st),
            Request::get("/v1/router/config").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(got.0, 200);
        let names = got.1["document"]["entrypoints"][0]["model_names"][0].as_str();
        assert_eq!(names, Some("ariacompute/semantic-renamed"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn topology_semantic_and_agent() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let (status, body) = oneshot_json(
            mgmt_router(st),
            Request::get("/v1/router/topology").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        let nodes = body["nodes"].as_array().unwrap();
        let kinds: Vec<&str> = nodes.iter().filter_map(|n| n["kind"].as_str()).collect();
        assert!(kinds.contains(&"entrypoint"));
        assert!(kinds.contains(&"recipe"));
        assert!(kinds.contains(&"signal"));
        assert!(kinds.contains(&"decision"));
        assert!(kinds.contains(&"model"));
        assert!(kinds.contains(&"builtin"));
        let ids: Vec<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
        assert!(ids.iter().any(|id| id.contains("needs_explain")));
        assert!(ids.contains(&"builtin:builtin"));
        assert!(ids.contains(&"model:local/general"));
    }

    #[tokio::test]
    async fn playground_chat_and_providers_list() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        st.set_fake_agent(
            "builtin",
            RouteDecision {
                model: "local/general".into(),
                algorithm: Some("static".into()),
                reason: "fake".into(),
                confidence: 0.9,
                layer: "agent".into(),
                decision: "agent".into(),
                bypass: false,
            },
        );
        let admin = mgmt_router(st.clone());
        let res = admin
            .oneshot(
                Request::post("/v1/router/chat")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "model": "ariacompute/semantic-auto",
                            "messages": [{"role":"user","content":"please explain rust"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.headers().get("x-aria-router-layer").unwrap(),
            "semantic"
        );

        let admin = mgmt_router(st.clone());
        let res = admin
            .oneshot(
                Request::put("/v1/router/providers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "engine/local",
                            "endpoint": backend,
                            "provider_model_id": "echo"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);

        let (status, body) = oneshot_json(
            mgmt_router(st),
            Request::get("/v1/router/providers").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        let names: Vec<&str> = body["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["name"].as_str())
            .collect();
        assert!(names.contains(&"local/general"));
        assert!(names.contains(&"engine/local"));
        let engine = body["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == "engine/local")
            .unwrap();
        assert_eq!(engine["backend_refs"][0]["endpoint"], backend);
    }

    #[tokio::test]
    async fn no_dashboard_root_is_404() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let res = mgmt_router(st)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_serves_index_when_dist_present() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dashboard/dist");
        if !dir.join("index.html").is_file() {
            return;
        }
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let res = mgmt_router_with_dashboard(st, dir)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let html = String::from_utf8_lossy(&bytes);
        assert!(html.contains("aria-router") || html.contains("root"));
    }

    #[tokio::test]
    async fn embedded_dashboard_serves_index() {
        if !crate::embed::has_dashboard() {
            return;
        }
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let res = crate::embed::mgmt_router_with_embedded_dashboard(st)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let html = String::from_utf8_lossy(&bytes);
        assert!(html.contains("aria-router") || html.contains("root"));
    }

    fn tiny_yaml_priced(backend: &str, require_key: bool) -> String {
        format!(
            r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/general
  models:
    - name: local/general
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      pricing:
        input_per_mtok: 1.0
        output_per_mtok: 2.0
      backend_refs:
        - name: primary
          endpoint: {backend}
          protocol: http
          weight: 100
entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      signals:
        keywords:
          - name: needs_explain
            operator: OR
            keywords: ["explain"]
      decisions:
        - name: explain
          priority: 10
          rules:
            operator: AND
            conditions:
              - type: keywords
                name: needs_explain
          modelRefs:
            - model: local/general
          algorithm: static
global:
  require_api_key: {require_key}
"#
        )
    }

    #[tokio::test]
    async fn cost_ledger_usage_and_factors() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml_priced(&backend, false)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "user": "alice",
            "messages": [{"role":"user","content":"please explain rust"}]
        });
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-aria-session", "s1")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let report = st.cost.lock().unwrap().report(10);
        assert_eq!(report["totals"]["requests"], 1);
        assert_eq!(report["totals"]["prompt_tokens"], 12);
        assert_eq!(report["totals"]["completion_tokens"], 3);
        assert!(report["totals"]["cost_usd"].as_f64().unwrap() > 0.0);
        assert!(report["factors"].get("users").is_some());
        assert_eq!(report["factors"]["users"], 1);
    }

    #[tokio::test]
    async fn api_key_required_401_and_by_key() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml_priced(&backend, true)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let created = st.keys.lock().unwrap().create("ci").unwrap();
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [{"role":"user","content":"please explain"}]
        });
        let res = app
            .clone()
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", created.secret))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let report = st.cost.lock().unwrap().report(5);
        let by_key = report["by_key"].as_object().unwrap();
        assert!(!by_key.is_empty());

        st.keys.lock().unwrap().revoke(&created.id).unwrap();
        let app2 = data_router(st);
        let res = app2
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", created.secret))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn keys_crud_and_provider_upsert_auth() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml_priced(&backend, true)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mgmt = mgmt_router(st.clone());
        let (status, created) = oneshot_json(
            mgmt.clone(),
            Request::post("/v1/router/keys")
                .header("content-type", "application/json")
                .body(Body::from(json!({"name":"eng"}).to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let secret = created["secret"].as_str().unwrap().to_string();
        let id = created["id"].as_str().unwrap().to_string();

        let res = mgmt
            .clone()
            .oneshot(
                Request::put("/v1/router/providers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "local/new",
                            "endpoint": backend,
                            "provider_model_id": "x",
                            "locality": "local"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = mgmt
            .clone()
            .oneshot(
                Request::put("/v1/router/providers")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::from(
                        json!({
                            "name": "local/new",
                            "endpoint": backend,
                            "provider_model_id": "x",
                            "locality": "local"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);

        let (status, _) = oneshot_json(
            mgmt,
            Request::delete(format!("/v1/router/keys/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    /// Spin up a mock Aria Compute serve that answers `GET /api/api-keys` with
    /// `status` and `body`.
    async fn spawn_serve(status: u16, body: Value) -> String {
        let app = Router::new().route(
            "/api/api-keys",
            axum::routing::get(move || {
                let body = body.clone();
                async move { (StatusCode::from_u16(status).unwrap(), Json(body)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    /// Link a serve account with a pasted `sk-bf-` key and point its site_url at
    /// `site_url` (bypassing `normalize_site` for tests).
    fn link_with_key(st: &Arc<AppState>, key: &str, site_url: &str) {
        let mut keys = st.keys.lock().unwrap();
        let (_, state, _) = keys
            .begin_link("com", Some("u1".into()), Some("a@b.com".into()), None)
            .unwrap();
        let _ = keys.take_pending(&state).unwrap();
        keys.apply_exchange(ExchangeInput {
            site: "https://ariacompute.com".into(),
            site_url: "https://ariacompute.com".into(),
            user: ServeUserInfo {
                id: serde_json::Value::String("s1".into()),
                email: Some("a@b.com".into()),
                role: Some("user".into()),
            },
            link_token: Some("lt".into()),
            expires_at: None,
            api_key: Some(("aria-router".into(), key.into())),
            owner_user_id: Some("u1".into()),
        })
        .unwrap();
        keys.set_site_url_for_test(site_url);
    }

    #[tokio::test]
    async fn serve_sync_detects_deleted_key_via_401() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let base = spawn_serve(401, json!({"error": "unauthorized"})).await;
        link_with_key(&st, "sk-bf-ABCDEFGHIJKLMNOP", &base);

        let app = mgmt_router(st.clone());
        let (status, body) = oneshot_json(
            app,
            Request::post("/v1/router/serve/account/sync")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["api_key_deleted"], true);
        assert_eq!(body["api_key_configured"], false);

        // Persisted: GET /serve/account reflects the deletion without network.
        let app2 = mgmt_router(st.clone());
        let (_, body2) = oneshot_json(
            app2,
            Request::get("/v1/router/serve/account")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(body2["api_key_deleted"], true);
    }

    #[tokio::test]
    async fn serve_sync_refreshes_meta_on_200_valid_bearer() {
        // A 200 listing means the bearer is still valid (the key exists on
        // serve). Serve now returns the plaintext secret, which the router stores
        // automatically (no manual paste); the displayed name/prefix refresh too.
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let base = spawn_serve(
            200,
            json!([{
                "name": "other",
                "prefix": "sk-bf-OTHERKEY12345",
                "secret": "sk-bf-OTHERKEY1234567890",
                "created_at": "2024-01-01T00:00:00Z"
            }]),
        )
        .await;
        link_with_key(&st, "sk-bf-ABCDEFGHIJKLMNOP", &base);

        let app = mgmt_router(st.clone());
        let (status, body) = oneshot_json(
            app,
            Request::post("/v1/router/serve/account/sync")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["api_key_deleted"], false);
        assert_eq!(body["api_key_configured"], true);
        // Metadata refreshed from the (only) serve key; secret auto-stored.
        assert_eq!(body["api_key_name"], "other");
        // The router can now authenticate with the auto-synced serve secret.
        assert!(st.keys.lock().unwrap().oauth_api_key().is_some());
    }

    #[tokio::test]
    async fn serve_sync_updates_meta_when_key_present() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let key = "sk-bf-ABCDEFGHIJKLMNOP";
        let base = spawn_serve(
            200,
            json!([{
                "name": "renamed",
                "prefix": key,
                "secret": key,
                "created_at": "2024-01-01T00:00:00Z"
            }]),
        )
        .await;
        link_with_key(&st, key, &base);

        let app = mgmt_router(st.clone());
        let (status, body) = oneshot_json(
            app,
            Request::post("/v1/router/serve/account/sync")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["api_key_deleted"], false);
        assert_eq!(body["api_key_configured"], true);
        assert_eq!(body["api_key_name"], "renamed");
    }

    #[tokio::test]
    async fn serve_sync_marks_deleted_when_empty_list() {
        // A 200 with an empty key list means the account has no usable key on
        // serve (all deleted/revoked). The router must clear the stale secret and
        // surface the degraded state — without a manual paste.
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let base = spawn_serve(200, json!([])).await;
        link_with_key(&st, "sk-bf-ABCDEFGHIJKLMNOP", &base);

        let app = mgmt_router(st.clone());
        let (status, body) = oneshot_json(
            app,
            Request::post("/v1/router/serve/account/sync")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["api_key_deleted"], true);
        assert_eq!(body["api_key_configured"], false);
        // The stale secret is cleared so the router stops authenticating with it.
        assert!(st.keys.lock().unwrap().oauth_api_key().is_none());
    }

    #[tokio::test]
    async fn serve_sync_adopts_newly_created_key() {
        // A newly created serve key (more recent created_at) is auto-synced: the
        // router adopts it and stores its secret, even when the prior key is still
        // present in the list.
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let old = "sk-bf-OLDKEY0000000000";
        let new_key = "sk-bf-NEWKEY0000000000";
        let base = spawn_serve(
            200,
            json!([
                {
                    "name": "old",
                    "prefix": old,
                    "secret": old,
                    "created_at": "2024-01-01T00:00:00Z"
                },
                {
                    "name": "fresh",
                    "prefix": new_key,
                    "secret": new_key,
                    "created_at": "2024-06-01T00:00:00Z"
                }
            ]),
        )
        .await;
        link_with_key(&st, old, &base);

        let app = mgmt_router(st.clone());
        let (status, body) = oneshot_json(
            app,
            Request::post("/v1/router/serve/account/sync")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["api_key_deleted"], false);
        assert_eq!(body["api_key_configured"], true);
        assert_eq!(body["api_key_name"], "fresh");
        // The router adopted the newer key's secret.
        assert_eq!(st.keys.lock().unwrap().oauth_api_key().as_deref(), Some(new_key));
    }
}
