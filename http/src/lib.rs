//! OpenAI data plane + management API + routing pipeline.

mod auth_api;
mod cost;
mod decision_ext;
mod embed;
pub(crate) mod keys;
mod memory;
mod ratelimit;
mod replay;
mod semantic_cache;
mod serve_account;
mod topology;
mod users;

use aria_router_afm_d::{task_from as afm_d_task_from, AfmDDecisioner};
use aria_router_agent::{request_view, task_from, BuiltinAgent, ToolRuntime};
use aria_router_algorithm::{global_elo, hard_filter, select, RuntimeStats};
use aria_router_config::{resolve_keys_path, resolve_users_path, Recipe, RouterDocument};
use aria_router_core::{ChatRequest, RouteDecision, RouterError, RouterKind};
use aria_router_decision::select_decision;
use aria_router_plugin::{
    apply_request, extra_headers, has_response_cache, remember_response, PluginHost, PluginOutcome,
};
use aria_router_provider::{forward, forward_sse_stream, PoolState, SseByteStream};
use aria_router_signal::extract;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use cost::{cost_usd, estimate_tokens, now_rfc3339, CostEvent, CostLedger};
pub use keys::{extract_bearer, validate_oauth_key, AuthIdentity, KeyStore};
use memory::SessionMemory;
use ratelimit::RateLimiter;
use replay::{ReplayLog, ReplayRecord};
use semantic_cache::SemanticCache;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use users::{UserRole, UserStore};

pub use embed::mgmt_router_serve_dashboard;
pub use users::UserStore as LocalUserStore;

pub struct AppState {
    pub doc: Mutex<RouterDocument>,
    pub config_path: PathBuf,
    pub pool: PoolState,
    pub plugins: PluginHost,
    pub last_route: Mutex<Option<RouteDecision>>,
    pub replay: Mutex<Vec<RouteDecision>>,
    pub replay_log: ReplayLog,
    pub decision_cache: aria_router_core::DecisionCache,
    pub competence: aria_router_core::CompetenceTracker,
    pub memory: SessionMemory,
    pub semantic_cache: SemanticCache,
    pub ratelimit: RateLimiter,
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
        let replay_log = ReplayLog::default();
        replay_log.configure(&doc.global.services.router_replay);
        Self {
            doc: Mutex::new(doc),
            config_path,
            pool: PoolState::default(),
            plugins: PluginHost::default(),
            last_route: Mutex::new(None),
            replay: Mutex::new(Vec::new()),
            replay_log,
            decision_cache: aria_router_core::DecisionCache::new(1024),
            competence: aria_router_core::CompetenceTracker::new(),
            memory: SessionMemory::default(),
            semantic_cache: SemanticCache::default(),
            ratelimit: RateLimiter::default(),
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
        .route(
            "/v1/router/auth/register-status",
            get(auth_api::register_status),
        )
        .route("/v1/router/auth/register", post(auth_api::register))
        .route("/v1/router/auth/login", post(auth_api::login))
        .route("/v1/router/auth/logout", post(auth_api::logout))
        .route("/v1/router/auth/me", get(auth_api::me))
        .route("/v1/router/auth/email", put(auth_api::set_my_email))
        .route("/v1/router/auth/password", post(auth_api::change_password))
        .route("/v1/router/users", get(auth_api::list_users))
        .route(
            "/v1/router/users/{id}/disabled",
            put(auth_api::set_user_disabled),
        )
        .route("/v1/router/users/{id}/email", put(auth_api::set_user_email))
        .route(
            "/v1/router/settings/allow_register",
            put(auth_api::set_allow_register),
        )
        .route("/v1/router/auth/oauth/start", post(auth_api::oauth_start))
        .route(
            "/v1/router/auth/oauth/callback",
            get(auth_api::oauth_callback),
        )
        .route("/v1/router/serve/account", get(auth_api::serve_account_get))
        .route(
            "/v1/router/serve/account/sync",
            post(auth_api::serve_account_sync),
        )
        .route("/v1/router/validate", post(validate_ep))
        .route("/v1/router/replay", get(replay_ep))
        .route("/v1/router/replay/{id}/reroute", post(replay_reroute_ep))
        .route(
            "/v1/router/providers",
            put(upsert_provider).get(list_providers),
        )
        .route("/v1/router/config", get(get_config).put(put_config))
        .route("/v1/router/overview", get(overview_ep))
        .route("/v1/router/topology", get(topology_ep))
        .route("/v1/router/models", get(list_models_mgmt))
        .route("/v1/router/chat", post(chat_mgmt))
        .route("/v1/router/cost", get(cost_ep))
        .route("/v1/router/cost/by-competence", get(cost_by_competence_ep))
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

async fn replay_ep(State(st): State<Arc<AppState>>, Query(q): Query<ReplayQ>) -> Json<Value> {
    let n = q.n.unwrap_or(20).min(100);
    let items = replay_items(&st, n);
    let records = st.replay_log.recent(n);
    Json(json!({"items": items, "records": records}))
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
struct RerouteQ {
    #[serde(default)]
    forward: bool,
}

async fn replay_reroute_ep(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<RerouteQ>,
) -> Result<Json<Value>, AppError> {
    let rec = st
        .replay_log
        .get(&id)
        .ok_or_else(|| AppError(RouterError::InvalidParam(format!("replay {id} not found"))))?;
    let preview = rec.prompt_preview.clone().unwrap_or_default();
    let model = rec.decision.model.clone();
    let entrypoint = {
        let doc = snapshot_doc(&st);
        // Prefer original entrypoint from decision layer path: use first semantic entry if any.
        doc.entrypoints
            .first()
            .and_then(|e| e.model_names.first().cloned())
            .unwrap_or_else(|| "ariacompute/semantic-auto".into())
    };
    let req = ChatRequest {
        model: entrypoint,
        messages: vec![aria_router_core::ChatMessage {
            role: "user".into(),
            content: Value::String(preview),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        extra: Default::default(),
    };
    let mut meta = HashMap::new();
    if let Some(s) = &rec.session {
        meta.insert("session".into(), s.clone());
    }
    let (decision, fwd, extra, _hdrs) = route_request(&st, req, &meta).await.map_err(AppError)?;
    if q.forward {
        let _ = model;
        let doc = snapshot_doc(&st);
        if let Some(body) = extra {
            return Ok(Json(
                json!({"decision": decision, "body": body, "forwarded": false}),
            ));
        }
        let body = forward(&doc, &decision.model, &fwd, &[], &st.pool)
            .await
            .map_err(AppError)?;
        return Ok(Json(
            json!({"decision": decision, "body": body, "forwarded": true}),
        ));
    }
    Ok(Json(json!({
        "decision": decision,
        "forwarded": false,
        "original_id": id,
    })))
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
    if let Some(existing) = doc
        .providers
        .models
        .iter_mut()
        .find(|m| m.name == body.name)
    {
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
        doc.providers
            .models
            .push(aria_router_config::ProviderModel {
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
                tier: None,
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
                "tier": m.tier,
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
        std::fs::write(&st.config_path, raw)
            .map_err(|e| RouterError::Io(format!("write {}: {e}", st.config_path.display())))?;
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

/// Cost attributed by competence tier, with estimated savings from competence-aware
/// downgrades.
async fn cost_by_competence_ep(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.cost.lock().unwrap().by_competence_tier())
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

async fn list_models_mgmt(State(st): State<Arc<AppState>>) -> Json<Value> {
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

/// Trusted in-process chat for SDK/FFI (not an HTTP route). Skips Bearer /
/// `require_api_key` — spoofable headers must never reach this path on `serve`.
pub async fn sdk_chat_complete(st: Arc<AppState>, body: Value) -> Result<Response, RouterError> {
    chat_with_auth(
        st,
        HeaderMap::new(),
        body,
        ChatAuth {
            user: "sdk".into(),
            key_id: None,
            key_name: None,
            identity: "sdk".into(),
            serve_user_id: None,
            serve_email: None,
            serve_site: None,
        },
    )
    .await
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
    match chat_with_auth(st, headers, body, auth).await {
        Ok(r) => r,
        Err(e) => AppError(e).into_response(),
    }
}

async fn chat_with_auth(
    st: Arc<AppState>,
    headers: HeaderMap,
    body: Value,
    auth: ChatAuth,
) -> Result<Response, RouterError> {
    let req: ChatRequest =
        serde_json::from_value(body).map_err(|e| RouterError::InvalidParam(e.to_string()))?;
    // ContextHold (prefix_reuse / sticky) only keys off an explicit client session.
    // Auto `req_*` ids are for cost ledger uniqueness and must not freeze decisions.
    let explicit = explicit_session(&headers, &req);
    let session = explicit.clone().unwrap_or_else(ephemeral_session_id);
    let mut metadata = metadata_from_headers(&headers);
    if let Some(s) = explicit {
        metadata.insert("session".into(), s);
    }
    let want_stream = req.stream;
    let entrypoint = req.model.clone();
    route_and_forward(
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

#[derive(Clone)]
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

/// Client-provided session for ContextHold (`x-aria-session` or body `session`).
fn explicit_session(headers: &HeaderMap, req: &ChatRequest) -> Option<String> {
    if let Some(v) = headers.get("x-aria-session").and_then(|v| v.to_str().ok()) {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Some(v) = req.extra.get("session").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

fn ephemeral_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!(
        "req_{}_{}",
        now_rfc3339(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

async fn route_and_forward(
    st: Arc<AppState>,
    req: ChatRequest,
    want_stream: bool,
    metadata: HashMap<String, String>,
    ctx: CostCtx,
) -> Result<Response, RouterError> {
    {
        let doc = snapshot_doc(&st);
        let bucket = ctx
            .key_id
            .clone()
            .unwrap_or_else(|| format!("anon:{}", ctx.user));
        st.ratelimit
            .check(&bucket, &doc.global.services.ratelimit)?;
    }
    let turns_in_request = req.messages.iter().filter(|m| m.role == "user").count() as u32;
    let prompt_est = estimate_tokens(&req.prompt_text());
    let last_user = req.last_user_text();
    let session = ctx.session.clone();
    let route_t0 = std::time::Instant::now();
    let (mut decision, mut fwd, extra, hdrs) = route_request(&st, req, &metadata).await?;
    if decision.routing_latency_ms.is_none() {
        decision.routing_latency_ms = Some(route_t0.elapsed().as_secs_f64() * 1000.0);
    }
    decision.session = Some(session.clone());
    // Semantic cache lookup (non-stream).
    if !want_stream && extra.is_none() {
        let doc = snapshot_doc(&st);
        if let Some(cached) = st.semantic_cache.lookup(
            &last_user,
            &decision.model,
            &decision.decision,
            &doc.global.stores.semantic_cache,
        ) {
            decision.semantic_cache_hit = Some(true);
            record(&st, decision.clone());
            let completion = estimate_tokens(&cached.to_string());
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
            let mut res = Json(cached).into_response();
            attach_route_headers(res.headers_mut(), &decision);
            return Ok(res);
        }
    }
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
            maybe_store_semantic(&st, &decision, &last_user, &fast);
            let mut res = Json(fast).into_response();
            attach_route_headers(res.headers_mut(), &decision);
            Ok(res)
        }
        None => {
            let doc = snapshot_doc(&st);
            if want_stream {
                fwd.stream = true;
                let sse = forward_sse_stream(&doc, &decision.model, &fwd, &hdrs, &st.pool).await?;
                let body = sse_response_body(
                    st.clone(),
                    sse,
                    decision.clone(),
                    ctx,
                    turns_in_request,
                    prompt_est,
                );
                let mut res = Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(body)
                    .unwrap();
                attach_route_headers(res.headers_mut(), &decision);
                Ok(res)
            } else {
                let body = forward(&doc, &decision.model, &fwd, &hdrs, &st.pool).await?;
                if decision.cache_response {
                    let cache_plugins = [aria_router_config::PluginRef {
                        name: "response-cache".into(),
                        extra: Default::default(),
                    }];
                    remember_response(&st.plugins, &cache_plugins, &fwd, &body);
                }
                maybe_store_semantic(&st, &decision, &last_user, &body);
                global_elo().observe_latency(
                    &decision.model,
                    decision.routing_latency_ms.unwrap_or(0.0) as f32,
                    &[],
                );
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

fn maybe_store_semantic(st: &AppState, decision: &RouteDecision, text: &str, body: &Value) {
    if decision.retention_drop == Some(true) {
        return;
    }
    let doc = snapshot_doc(st);
    let cfg = &doc.global.stores.semantic_cache;
    if !cfg.enabled {
        return;
    }
    let ttl = decision.retention_ttl_turns.unwrap_or(cfg.ttl_turns).max(1);
    st.semantic_cache.store(
        text,
        &decision.model,
        &decision.decision,
        body.clone(),
        ttl,
        cfg,
    );
}

/// Stream upstream SSE bytes to the client; record pool latency + cost when the body finishes.
fn sse_response_body(
    st: Arc<AppState>,
    sse: SseByteStream,
    decision: RouteDecision,
    ctx: CostCtx,
    turns_in_request: u32,
    prompt_est: u64,
) -> Body {
    use futures_util::StreamExt;
    let stream = SseCostStream {
        inner: sse,
        st,
        decision,
        ctx,
        turns_in_request,
        prompt_est,
        recorded: false,
    };
    Body::from_stream(stream.map(|item| item.map_err(|e| std::io::Error::other(e.to_string()))))
}

struct SseCostStream {
    inner: SseByteStream,
    st: Arc<AppState>,
    decision: RouteDecision,
    ctx: CostCtx,
    turns_in_request: u32,
    prompt_est: u64,
    recorded: bool,
}

impl SseCostStream {
    fn finish(&mut self, ok: bool) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        self.st
            .pool
            .record(self.inner.model(), self.inner.elapsed_ms(), ok);
        let text = self.inner.accumulated();
        let (pt, ct, src) = parse_sse_usage(&text, self.prompt_est);
        record_cost(
            &self.st,
            &self.ctx,
            &self.decision,
            CostUsage {
                turns_in_request: self.turns_in_request,
                upstream_requests: 1,
                prompt_tokens: pt,
                completion_tokens: ct,
                tokens_source: src,
            },
        );
    }
}

impl futures_util::Stream for SseCostStream {
    type Item = Result<bytes::Bytes, RouterError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        match std::pin::Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(b))) => Poll::Ready(Some(Ok(b))),
            Poll::Ready(Some(Err(e))) => {
                self.finish(false);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                self.finish(true);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

fn parse_json_usage(body: &Value, prompt_est: u64) -> (u64, u64, &'static str) {
    if let Some(u) = body.get("usage") {
        let pt = u
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(prompt_est);
        let ct = u
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
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
                let pt = u
                    .get("prompt_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(prompt_est);
                let ct = u
                    .get("completion_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                return (pt, ct, "usage");
            }
        }
    }
    (prompt_est, estimate_tokens(text) / 4, "estimate")
}

fn record_cost(st: &AppState, ctx: &CostCtx, decision: &RouteDecision, usage: CostUsage<'_>) {
    let doc = snapshot_doc(st);
    let (in_p, out_p, priced) = match doc
        .provider(&decision.model)
        .and_then(|p| p.pricing.as_ref())
    {
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
        competence_tier: Some(aria_router_core::tier_of_model(&decision.model).to_string()),
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
    let _ = h.insert(
        "x-aria-router-layer",
        d.layer.parse().unwrap_or_else(|_| "none".parse().unwrap()),
    );
    let _ = h.insert(
        "x-aria-router-decision",
        d.decision
            .parse()
            .unwrap_or_else(|_| "none".parse().unwrap()),
    );
    if let Ok(v) = d.model.parse() {
        h.insert("x-aria-router-model", v);
    }
    if let Some(algo) = &d.algorithm {
        if let Ok(v) = algo.parse() {
            h.insert("x-aria-router-algorithm", v);
        }
    }
    if !d.reason.is_empty() {
        let reason: String = d
            .reason
            .chars()
            .map(|c| {
                if c.is_ascii_graphic() || c == ' ' {
                    c
                } else {
                    '?'
                }
            })
            .take(200)
            .collect();
        if let Ok(v) = reason.parse() {
            h.insert("x-aria-router-reason", v);
        }
    }
    let conf = format!("{:.4}", d.confidence);
    if let Ok(v) = conf.parse() {
        h.insert("x-aria-router-confidence", v);
    }
    let bypass = if d.bypass { "true" } else { "false" };
    if let Ok(v) = bypass.parse() {
        h.insert("x-aria-router-bypass", v);
    }
    if let Some(ms) = d.routing_latency_ms {
        let s = format!("{ms:.3}");
        if let Ok(v) = s.parse() {
            h.insert("x-aria-router-latency-ms", v);
        }
    }
    if let Some(v) = d.retention_drop {
        if let Ok(hv) = (if v { "true" } else { "false" }).parse() {
            h.insert("x-aria-router-retention-drop", hv);
        }
    }
    if let Some(t) = d.retention_ttl_turns {
        if let Ok(hv) = t.to_string().parse() {
            h.insert("x-aria-router-retention-ttl-turns", hv);
        }
    }
    if let Some(v) = d.retention_keep_current_model {
        if let Ok(hv) = (if v { "true" } else { "false" }).parse() {
            h.insert("x-aria-router-retention-keep-current-model", hv);
        }
    }
    if let Some(v) = d.retention_prefer_prefix {
        if let Ok(hv) = (if v { "true" } else { "false" }).parse() {
            h.insert("x-aria-router-retention-prefer-prefix", hv);
        }
    }
    if d.semantic_cache_hit == Some(true) {
        if let Ok(hv) = "hit".parse() {
            h.insert("x-aria-router-semantic-cache", hv);
        }
    }
}

fn record(st: &AppState, d: RouteDecision) {
    *st.last_route.lock().unwrap() = Some(d.clone());
    let mut r = st.replay.lock().unwrap();
    r.push(d.clone());
    if r.len() > 256 {
        r.remove(0);
    }
    let doc = snapshot_doc(st);
    let cfg = doc.global.services.router_replay.clone();
    let selected_group = aria_router_core::tier_of_model(&d.model).to_string();
    st.replay_log.push(
        ReplayRecord {
            id: d.replay_id.clone().unwrap_or_default(),
            decision: d,
            signals_summary: None,
            projections: None,
            emits: None,
            retention: None,
            session: None,
            prompt_preview: None,
            task_category: None,
            selected_group: Some(selected_group),
            tool_outcomes: None,
            accepted_output: None,
            stage_competence: None,
        },
        &cfg,
    );
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
) -> Result<
    (
        RouteDecision,
        ChatRequest,
        Option<Value>,
        Vec<(String, String)>,
    ),
    RouterError,
> {
    let doc = snapshot_doc(st);
    if doc.is_concrete_model(&req.model) {
        let d = RouteDecision::bypass(&req.model);
        return Ok((d, req, None, vec![]));
    }
    let ep = doc
        .entrypoint_for(&req.model)
        .ok_or_else(|| RouterError::FailClosed(format!("unknown model {}", req.model)))?;
    let recipe = doc.recipe(&ep.recipe)?.clone();
    if recipe.router != ep.router {
        return Err(RouterError::Config(
            "entrypoint/recipe router mismatch".into(),
        ));
    }
    // ContextHold / prefix reuse: freeze the decision within a user turn;
    // re-decide on a new user message or after context compaction.
    let session = metadata.get("session").cloned().unwrap_or_default();
    let turn = aria_router_core::turn_of(&req.messages);
    let tools = req.extra.get("tools");
    let prefix_reuse = match ep.router {
        RouterKind::Semantic => recipe
            .routing
            .as_ref()
            .and_then(|r| r.prefix_reuse)
            .unwrap_or(true),
        RouterKind::Agent => recipe
            .agent
            .as_ref()
            .and_then(|a| a.prefix_reuse)
            .unwrap_or(true),
        RouterKind::AfmD => recipe
            .afm_d
            .as_ref()
            .and_then(|a| a.prefix_reuse)
            .unwrap_or(true),
    };
    let memory_on = doc.global.stores.memory.enabled;
    if prefix_reuse && !session.is_empty() {
        let compacted = st
            .decision_cache
            .observe_and_check_compact(&session, &req.messages, tools);
        if compacted {
            st.memory.clear(&session);
        } else {
            st.memory.tick_if_new_turn(&session, turn);
        }
        if let Some(cached) = st.decision_cache.get(&session, turn) {
            return Ok((cached, req, None, vec![]));
        }
        // afm-d / agent: sticky short-circuit on a new user turn when still eligible.
        if matches!(ep.router, RouterKind::Agent | RouterKind::AfmD) && memory_on {
            if let Some(st_mem) = st.memory.get(&session) {
                let all = hard_filter(
                    &doc,
                    std::slice::from_ref(&st_mem.sticky_model),
                    None,
                    Some("text"),
                );
                if let Some(card) = all.into_iter().next() {
                    let decision = RouteDecision {
                        model: card.name.clone(),
                        algorithm: None,
                        reason: format!("sticky:{}", st_mem.decision),
                        confidence: 1.0,
                        layer: ep.router.as_str().into(),
                        decision: st_mem.decision.clone(),
                        bypass: false,
                        routing_latency_ms: None,
                        cache_response: false,
                        retention_drop: None,
                        retention_ttl_turns: Some(st_mem.ttl_turns_left),
                        retention_keep_current_model: Some(true),
                        retention_prefer_prefix: None,
                        semantic_cache_hit: None,
                        replay_id: None,
                        session: Some(session.clone()),
                    };
                    st.decision_cache
                        .put(session.clone(), turn, decision.clone());
                    let mut fwd = req;
                    fwd.model = decision.model.clone();
                    return Ok((decision, fwd, None, vec![]));
                }
            }
        }
    }
    let (mut decision, fwd, extra, hdrs) = match ep.router {
        RouterKind::Semantic => route_semantic(st, &doc, &recipe, req, metadata).await,
        RouterKind::Agent => route_agent(st, &doc, &recipe, req, metadata).await,
        RouterKind::AfmD => route_afm_d(st, &doc, &recipe, req, metadata).await,
    }?;
    if prefix_reuse && !session.is_empty() {
        st.decision_cache
            .put(session.clone(), turn, decision.clone());
        // afm-d / agent pin sticky so the next user turn can short-circuit.
        if matches!(ep.router, RouterKind::Agent | RouterKind::AfmD) && memory_on {
            let ttl = doc.global.stores.memory.default_ttl_turns.max(1);
            st.memory.pin(
                &session,
                decision.model.clone(),
                decision.decision.clone(),
                ttl,
                turn,
                &doc.global.stores.memory,
            );
            decision.retention_keep_current_model = Some(true);
            decision.retention_ttl_turns = Some(ttl);
            decision.session = Some(session);
        }
    }
    Ok((decision, fwd, extra, hdrs))
}

async fn route_semantic(
    st: &AppState,
    doc: &RouterDocument,
    recipe: &Recipe,
    req: ChatRequest,
    metadata: &HashMap<String, String>,
) -> Result<
    (
        RouteDecision,
        ChatRequest,
        Option<Value>,
        Vec<(String, String)>,
    ),
    RouterError,
> {
    #[cfg(not(feature = "ml"))]
    {
        let learned = doc.learned_signal_referenced(recipe);
        if !learned.is_empty() {
            return Err(RouterError::Unsupported(format!(
                "learned signal {} requires feature ml / weights",
                learned[0]
            )));
        }
    }
    let routing = recipe
        .routing
        .as_ref()
        .ok_or_else(|| RouterError::Config("missing routing".into()))?;
    let signals = extract(doc, recipe, &req, metadata)?;
    let proj = aria_router_decision::project(&routing.projections, &signals)?;
    let decision_cfg = select_decision(recipe, &signals, &proj, &routing.strategy)?;
    let retention = decision_cfg.as_ref().and_then(|d| d.retention().cloned());
    let (model_names, algo, plugins, dname, loc, emits) = if let Some(d) = &decision_cfg {
        (
            d.model_refs
                .iter()
                .map(|m| m.model.clone())
                .collect::<Vec<_>>(),
            d.algorithm.clone(),
            d.plugins.clone(),
            d.name.clone(),
            d.locality.clone(),
            d.emits.clone(),
        )
    } else {
        let def = doc.providers.defaults.default_model.clone();
        if def.is_empty() {
            return Err(RouterError::FailClosed(
                "no matching decision and no default_model".into(),
            ));
        }
        (
            vec![def],
            Some("static".into()),
            vec![],
            "default".into(),
            None,
            vec![],
        )
    };
    let eligible = hard_filter(doc, &model_names, loc.as_deref(), Some("text"));
    if eligible.is_empty() {
        return Err(RouterError::FailClosed(
            "no eligible models after hard constraints".into(),
        ));
    }
    let session = metadata.get("session").cloned().unwrap_or_default();
    // Sticky model from prior retention (must still pass hard constraints).
    let sticky_override = if doc.global.stores.memory.enabled && !session.is_empty() {
        if let Some(st_mem) = st.memory.get(&session) {
            let all = hard_filter(
                doc,
                std::slice::from_ref(&st_mem.sticky_model),
                None,
                Some("text"),
            );
            all.into_iter().next().map(|c| c.name)
        } else {
            None
        }
    } else {
        None
    };
    let dummy = aria_router_config::DecisionCfg {
        name: dname.clone(),
        description: None,
        priority: 0,
        rules: Default::default(),
        model_refs: eligible
            .iter()
            .map(|e| aria_router_config::ModelRef {
                model: e.name.clone(),
            })
            .collect(),
        algorithm: algo.clone(),
        plugins: plugins.clone(),
        locality: loc,
        emits: emits.clone(),
    };
    let algo_name = algo.as_deref().unwrap_or("static");
    let stats = if algo_name == "static" && sticky_override.is_none() {
        RuntimeStats::default()
    } else {
        let mut cost_map = HashMap::new();
        for m in &doc.providers.models {
            cost_map.insert(m.name.clone(), m.ranking_cost());
        }
        let mut elo_map = HashMap::new();
        for e in &eligible {
            elo_map.insert(e.name.clone(), global_elo().rating(&e.name));
        }
        RuntimeStats {
            latency_ms: st.pool.latency_map(),
            cost: cost_map,
            elo: elo_map,
            ..Default::default()
        }
    };
    let mut model = select(doc, &dummy, &eligible, &stats)?;
    if let Some(sticky) = sticky_override {
        model = sticky;
    }
    // Competence-aware downgrade: if the task is simple enough and
    // historical competence is above the floor, prefer the cheapest eligible tier.
    // `simple_prob` is approximated by the strongest projection score; `competence`
    // comes from the per-session tracker (None ⇒ unknown ⇒ no downgrade).
    if let Some(comp_cfg) = &routing.competence {
        let simple_prob = proj
            .values
            .values()
            .filter_map(|v| v.as_f64())
            .fold(0.0_f64, f64::max) as f32;
        let competence = st.competence.score(&session);
        if let Some(down) = aria_router_decision::apply_competence(
            &model,
            &eligible,
            simple_prob,
            competence,
            comp_cfg,
        ) {
            model = down;
        }
    }
    // Decision-model extension point: an external decision service
    // may override the model choice. Unreachable / malformed → fallback model or
    // FailClosed via the extension client.
    let mut ext_meta: Option<RouteDecision> = None;
    if let Some(ext) = &routing.decision_ext {
        match decision_ext::decide(&req, ext).await {
            Ok(d) => {
                model = d.model.clone();
                ext_meta = Some(d);
            }
            Err(e) => return Err(e),
        }
    }
    let hdrs = extra_headers(&plugins);
    let cache_response = has_response_cache(&plugins);
    let mut req_for_plugins = req.clone();
    req_for_plugins.model = model.clone();
    let keep = retention
        .as_ref()
        .and_then(|r| r.keep_current_model)
        .unwrap_or(false);
    let ttl = retention
        .as_ref()
        .and_then(|r| r.ttl_turns)
        .unwrap_or(doc.global.stores.memory.default_ttl_turns);
    let user_turn = aria_router_core::turn_of(&req.messages);
    if keep && doc.global.stores.memory.enabled && !session.is_empty() {
        // Same model re-pin keeps remaining TTL (ContextHold: no refresh).
        st.memory.pin(
            &session,
            model.clone(),
            dname.clone(),
            ttl.max(1),
            user_turn,
            &doc.global.stores.memory,
        );
    }
    let signals_summary = json!(signals
        .hits
        .iter()
        .map(|h| json!({"kind": h.kind, "name": h.name, "matched": h.matched, "confidence": h.confidence}))
        .collect::<Vec<_>>());
    let decision =
        |model: String, algo: Option<String>, dname: String, cache_response: bool| RouteDecision {
            model,
            algorithm: algo,
            reason: format!("semantic:{dname}"),
            confidence: 1.0,
            layer: "semantic".into(),
            decision: dname,
            bypass: false,
            routing_latency_ms: None,
            cache_response,
            retention_drop: retention.as_ref().and_then(|r| r.drop),
            retention_ttl_turns: retention.as_ref().and_then(|r| r.ttl_turns),
            retention_keep_current_model: retention.as_ref().and_then(|r| r.keep_current_model),
            retention_prefer_prefix: retention.as_ref().and_then(|r| r.prefer_prefix_retention),
            semantic_cache_hit: None,
            replay_id: None,
            session: if session.is_empty() {
                None
            } else {
                Some(session.clone())
            },
        };
    // Enrich replay with signal/projection context on next record() via last fields —
    // stash on decision.session already; push detailed record here.
    let cfg = doc.global.services.router_replay.clone();
    let _ = (&signals_summary, &proj, &emits, &cfg);
    // Apply decision-model extension override (if any) to the final decision metadata.
    let (dname, algo) = if let Some(ext) = &ext_meta {
        (ext.decision.clone(), ext.algorithm.clone())
    } else {
        (dname.clone(), algo.clone())
    };
    match apply_request(&st.plugins, &plugins, req_for_plugins)? {
        PluginOutcome::FastResponse(v) => {
            let d = decision(model.clone(), algo, dname, cache_response);
            push_rich_replay(st, &d, &signals_summary, &proj, &emits, &retention, &req);
            Ok((
                d,
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
            ))
        }
        PluginOutcome::Continue(fwd) => {
            let d = decision(model.clone(), algo, dname, cache_response);
            push_rich_replay(st, &d, &signals_summary, &proj, &emits, &retention, &req);
            Ok((
                d,
                {
                    let mut f = fwd;
                    f.model = model;
                    f
                },
                None,
                hdrs,
            ))
        }
    }
}

fn push_rich_replay(
    st: &AppState,
    d: &RouteDecision,
    signals: &Value,
    proj: &aria_router_decision::ProjectionMap,
    emits: &[aria_router_config::EmitDirective],
    retention: &Option<aria_router_config::RetentionDirective>,
    req: &ChatRequest,
) {
    let doc = snapshot_doc(st);
    let cfg = &doc.global.services.router_replay;
    st.replay_log.push(
        ReplayRecord {
            id: String::new(),
            decision: d.clone(),
            signals_summary: Some(signals.clone()),
            projections: serde_json::to_value(&proj.values).ok(),
            emits: serde_json::to_value(emits).ok(),
            retention: retention.clone(),
            session: d.session.clone(),
            prompt_preview: Some(req.last_user_text().chars().take(200).collect()),
            task_category: None,
            selected_group: Some(aria_router_core::tier_of_model(&d.model).to_string()),
            tool_outcomes: None,
            accepted_output: None,
            stage_competence: None,
        },
        cfg,
    );
}

async fn route_afm_d(
    _st: &AppState,
    doc: &RouterDocument,
    recipe: &Recipe,
    req: ChatRequest,
    _metadata: &HashMap<String, String>,
) -> Result<
    (
        RouteDecision,
        ChatRequest,
        Option<Value>,
        Vec<(String, String)>,
    ),
    RouterError,
> {
    let afm = recipe
        .afm_d
        .as_ref()
        .ok_or_else(|| RouterError::Config("missing afm-d".into()))?;
    let all_names: Vec<String> = doc
        .providers
        .models
        .iter()
        .map(|m| m.name.clone())
        .collect();
    let eligible = hard_filter(doc, &all_names, None, Some("text"));
    if eligible.is_empty() {
        return Err(RouterError::FailClosed(
            "no eligible models after hard constraints".into(),
        ));
    }
    let task = afm_d_task_from(&req, eligible.clone(), afm);
    let decisioner = AfmDDecisioner {
        endpoint: afm.endpoint.clone(),
    };
    let mut decision = decisioner.route(task).await?;
    decision.layer = "afm-d".into();
    if let Some(fb) = &afm.fallback {
        if !eligible.iter().any(|e| e.name == decision.model) {
            decision.model = fb.clone();
            decision.reason = format!("fallback:{fb}");
        }
    }
    if !eligible.iter().any(|e| e.name == decision.model) && doc.provider(&decision.model).is_none()
    {
        return Err(RouterError::FailClosed(format!(
            "afm-d model {} not eligible",
            decision.model
        )));
    }
    let mut fwd = req;
    fwd.model = decision.model.clone();
    Ok((decision, fwd, None, vec![]))
}

async fn route_agent(
    st: &AppState,
    doc: &RouterDocument,
    recipe: &Recipe,
    req: ChatRequest,
    _metadata: &HashMap<String, String>,
) -> Result<
    (
        RouteDecision,
        ChatRequest,
        Option<Value>,
        Vec<(String, String)>,
    ),
    RouterError,
> {
    let agent = recipe
        .agent
        .as_ref()
        .ok_or_else(|| RouterError::Config("missing agent".into()))?;
    let all_names: Vec<String> = doc
        .providers
        .models
        .iter()
        .map(|m| m.name.clone())
        .collect();
    // Agent may route to cloud or local backends; do not hard-require locality=local.
    let eligible = hard_filter(doc, &all_names, None, Some("text"));
    if eligible.is_empty() {
        return Err(RouterError::FailClosed(
            "no eligible models after hard constraints".into(),
        ));
    }
    // Long-horizon agent delegation: route the decision to an external
    // agent / Worker-plan endpoint instead of the in-process builtin. Unreachable /
    // timeout → fallback model or FailClosed via the decision-ext contract.
    if let Some(ep) = &agent.delegate_endpoint {
        let ext = aria_router_config::DecisionExtCfg {
            endpoint: Some(ep.clone()),
            track: agent.worker_plan.clone(),
            timeout_ms: agent.timeout_ms,
            fallback: agent.fallback.clone(),
            auth: agent.delegate_auth.clone(),
        };
        let mut d = decision_ext::decide(&req, &ext).await?;
        d.layer = "agent-delegate".into();
        let mut fwd = req;
        fwd.model = d.model.clone();
        return Ok((d, fwd, None, vec![]));
    }
    let task = task_from(&req, eligible.clone(), agent);
    let tools = ToolRuntime {
        latency_ms: st.pool.latency_map(),
        failures: st.pool.failures_map(),
        request_view: request_view(&req),
    };
    let canned = st.fake_agents.lock().unwrap().get("builtin").cloned();
    let logical_llm = agent.model.clone().unwrap_or_else(|| "router-llm".into());
    let llm_provider = doc.provider(&logical_llm);
    let upstream_llm = llm_provider
        .map(|p| {
            if p.provider_model_id.is_empty() {
                p.name.clone()
            } else {
                p.provider_model_id.clone()
            }
        })
        .unwrap_or_else(|| logical_llm.clone());
    let api_key = llm_provider.and_then(|p| {
        p.backend_refs
            .iter()
            .max_by_key(|b| b.weight)
            .and_then(aria_router_provider::backend_api_key)
    });
    let builtin = BuiltinAgent {
        endpoint: agent.endpoint.clone(),
        model: upstream_llm,
        api_key,
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
            RouterError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            RouterError::FailClosed(_) => StatusCode::FORBIDDEN,
            RouterError::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
            RouterError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            RouterError::Config(_) | RouterError::InvalidParam(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::BAD_GATEWAY,
        };
        let body =
            json!({"error": {"message": self.0.to_string(), "type": format!("{:?}", self.0)}});
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

    /// Gold-path Gateway recipe: explain → large; systems/trade-off → mid; else → small.
    fn gateway_yaml(backend: &str) -> String {
        format!(
            r#"version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: ariacompute/ariamodel-mid
  models:
    - name: ariacompute/ariamodel-small
      provider_model_id: echo-small
      locality: cloud
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: {backend}
    - name: ariacompute/ariamodel-mid
      provider_model_id: echo-mid
      locality: cloud
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: {backend}
    - name: ariacompute/ariamodel-large
      provider_model_id: echo-large
      locality: cloud
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: {backend}
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
            keywords: ["explain", "walk me through"]
          - name: needs_mid
            operator: OR
            keywords: ["reverse proxy", "trade-off", "tradeoff", "architecture"]
        context:
          - name: long_prompt
            min_tokens: 256
            max_tokens: 8192
        conversation:
          - name: multi_turn
            min_messages: 2
        structure:
          - name: multi_question
            min_questions: 2
      decisions:
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: ariacompute/ariamodel-large
          algorithm: static
        - name: long_context
          priority: 80
          rules:
            operator: AND
            conditions:
              - type: context
                name: long_prompt
          modelRefs:
            - model: ariacompute/ariamodel-small
            - model: ariacompute/ariamodel-mid
            - model: ariacompute/ariamodel-large
          algorithm: latency-aware
        - name: systems_mid
          priority: 70
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_mid
          modelRefs:
            - model: ariacompute/ariamodel-mid
          algorithm: static
        - name: multi_turn
          priority: 60
          rules:
            operator: AND
            conditions:
              - type: conversation
                name: multi_turn
          modelRefs:
            - model: ariacompute/ariamodel-mid
          algorithm: static
        - name: multi_question
          priority: 50
          rules:
            operator: AND
            conditions:
              - type: structure
                name: multi_question
          modelRefs:
            - model: ariacompute/ariamodel-mid
          algorithm: static
        - name: fallback_pool
          priority: 1
          rules:
            operator: AND
            conditions: []
          modelRefs:
            - model: ariacompute/ariamodel-small
          algorithm: static
global:
  require_api_key: false
"#
        )
    }

    async fn mock_upstream() -> String {
        use axum::response::IntoResponse;
        use axum::routing::post;
        async fn echo(Json(v): Json<Value>) -> Response {
            let model = v.get("model").cloned().unwrap_or(json!("echo"));
            let stream = v.get("stream").and_then(|x| x.as_bool()).unwrap_or(false);
            if stream {
                let chunk = json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": {"role": "assistant", "content": "ok"},
                        "finish_reason": null
                    }]
                });
                let usage = json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 12,
                        "completion_tokens": 3,
                        "total_tokens": 15
                    }
                });
                let body = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", chunk, usage);
                Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(body))
                    .unwrap()
            } else {
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
                .into_response()
            }
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
    async fn gateway_hi_routes_to_small_not_large() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&gateway_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        // Seed latency favoring large — must not steal the static fallback gold path.
        st.pool.record("ariacompute/ariamodel-large", 5.0, true);
        st.pool.record("ariacompute/ariamodel-small", 500.0, true);
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/semantic-auto",
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
            res.headers().get("x-aria-router-model").unwrap(),
            "ariacompute/ariamodel-small"
        );
        assert_eq!(
            res.headers().get("x-aria-router-decision").unwrap(),
            "fallback_pool"
        );

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
            res.headers().get("x-aria-router-model").unwrap(),
            "ariacompute/ariamodel-large"
        );
    }

    #[tokio::test]
    async fn gateway_hi_after_explain_not_stuck_on_large() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&gateway_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st);
        // Prior user "explain" must not keep keyword sticky; multi-turn → mid.
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [
                {"role":"user","content":"explain C/C++/Rust/Go"},
                {"role":"assistant","content":"…"},
                {"role":"user","content":"hi"}
            ]
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
            res.headers().get("x-aria-router-decision").unwrap(),
            "multi_turn"
        );
        assert_eq!(
            res.headers().get("x-aria-router-model").unwrap(),
            "ariacompute/ariamodel-mid"
        );
    }

    #[tokio::test]
    async fn gateway_systems_mid_routes_reverse_proxy() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&gateway_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st);
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [{"role":"user","content":"Name one benefit of using an HTTP reverse proxy."}]
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
            res.headers().get("x-aria-router-decision").unwrap(),
            "systems_mid"
        );
        assert_eq!(
            res.headers().get("x-aria-router-model").unwrap(),
            "ariacompute/ariamodel-mid"
        );
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
        assert_eq!(res.headers().get("x-aria-router-layer").unwrap(), "bypass");
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
                routing_latency_ms: None,
                cache_response: false,
                ..Default::default()
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
        assert_eq!(
            res.headers().get("x-aria-router-layer").unwrap(),
            "semantic"
        );
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
                routing_latency_ms: None,
                cache_response: false,
                ..Default::default()
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
        assert!(res.headers().get("x-aria-router-model").is_some());
        assert!(
            res.headers().get("x-aria-router-algorithm").is_some()
                || res.headers().get("x-aria-router-reason").is_some()
                || res.headers().get("x-aria-router-confidence").is_some()
        );
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("data:"), "expected SSE chunks, got {text}");
        assert!(text.contains("ok"), "expected content in SSE, got {text}");

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

    #[tokio::test]
    async fn mgmt_models_lists_entrypoints() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let (status, body) = oneshot_json(
            mgmt_router(st),
            Request::get("/v1/router/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        let data = body["data"].as_array().unwrap();
        let ids: Vec<&str> = data.iter().filter_map(|m| m["id"].as_str()).collect();
        assert!(ids.contains(&"ariacompute/semantic-auto"));
        assert!(ids.contains(&"local/general"));
    }

    #[tokio::test]
    async fn playground_stream_passthrough() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let admin = mgmt_router(st);
        let res = admin
            .oneshot(
                Request::post("/v1/router/chat")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "model": "ariacompute/semantic-auto",
                            "stream": true,
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
            res.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        assert_eq!(
            res.headers().get("x-aria-router-layer").unwrap(),
            "semantic"
        );
        assert!(res.headers().get("x-aria-router-confidence").is_some());
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("data:"));
        assert!(text.contains("[DONE]") || text.contains("ok"));
    }

    async fn oneshot_json(app: Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let v = if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(json!({"raw": String::from_utf8_lossy(&bytes)}))
        };
        (status, v)
    }

    #[tokio::test]
    async fn put_config_rejects_unknown_block() {
        let backend = mock_upstream().await;
        let yaml = tiny_yaml(&backend);
        let path =
            std::env::temp_dir().join(format!("aria-router-put-bad-{}.yml", std::process::id()));
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
        assert_eq!(
            snapshot_doc(&st).entrypoints.len(),
            before.entrypoints.len()
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn put_config_reloads_tiny_yaml() {
        let backend = mock_upstream().await;
        let yaml = tiny_yaml(&backend);
        let path =
            std::env::temp_dir().join(format!("aria-router-put-ok-{}.yml", std::process::id()));
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
            Request::get("/v1/router/config")
                .body(Body::empty())
                .unwrap(),
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
            Request::get("/v1/router/topology")
                .body(Body::empty())
                .unwrap(),
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
                routing_latency_ms: None,
                cache_response: false,
                ..Default::default()
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
            Request::get("/v1/router/providers")
                .body(Body::empty())
                .unwrap(),
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

        // Trusted SDK path must not require Bearer when require_api_key is true.
        let sdk_res = sdk_chat_complete(
            st.clone(),
            json!({
                "model": "ariacompute/semantic-auto",
                "messages": [{"role":"user","content":"please explain"}]
            }),
        )
        .await
        .unwrap();
        assert_eq!(sdk_res.status(), 200);

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
        assert_eq!(
            st.keys.lock().unwrap().oauth_api_key().as_deref(),
            Some(new_key)
        );
    }

    #[tokio::test]
    async fn retention_sticky_same_session() {
        let backend = mock_upstream().await;
        let host = backend.trim_start_matches("http://");
        let yaml = include_str!("../../config/examples/semantic-stateful.yaml")
            .replace("127.0.0.1:8000", host)
            .replace("127.0.0.1:8001", host);
        let doc = RouterDocument::from_yaml_str(&yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-sticky-1".into());
        let req1 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(&st, req1, &meta).await.unwrap();
        assert_eq!(d1.model, "local/general");
        assert_eq!(d1.retention_keep_current_model, Some(true));
        // Second user turn (accumulating history) without keyword would hit
        // default_fast; sticky keeps general. Turn key advances so DecisionCache misses.
        let req2 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain rust"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("ok"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("hi"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d2, _, _, _) = route_request(&st, req2, &meta).await.unwrap();
        assert_eq!(d2.model, "local/general");
    }

    #[tokio::test]
    async fn cost_by_competence_reports_tiers_after_routing() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml_priced(&backend, false)).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
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
        let json = cost_by_competence_ep(axum::extract::State(st.clone())).await;
        let v = json.0;
        // local/general has no explicit tier → bucketed as "unknown".
        assert!(
            v["by_tier"].get("unknown").is_some(),
            "expected an unknown-tier bucket, got {v}"
        );
        assert!(v["estimated_savings_usd"].as_f64().unwrap() >= 0.0);
    }

    #[tokio::test]
    async fn hard_constraints_fail_closed_on_modality_mismatch() {
        // A matched decision references a model whose modality ('audio') cannot satisfy
        // the request's required modality ('text'). hard_filter therefore yields an empty
        // eligible set and routing fails closed.
        let doc = RouterDocument::from_yaml_str(
            r#"
version: v0.3
providers:
  defaults:
    default_model: local/general
  models:
    - name: local/general
      locality: local
      modality: audio
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: r
recipes:
  - name: r
    router: semantic
    routing:
      strategy: priority
      decisions:
        - name: d
          priority: 100
          rules: { operator: AND, conditions: [] }
          modelRefs: [{model: local/general}]
"#,
        )
        .unwrap();
        let (st, _dir) = isolated_state(doc);
        let meta = std::collections::HashMap::new();
        let req = aria_router_core::ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("hi"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let err = route_request(&st, req, &meta).await.unwrap_err();
        match err {
            aria_router_core::RouterError::FailClosed(msg) => assert!(
                msg.contains("no eligible models after hard constraints"),
                "unexpected fail-closed message: {msg}"
            ),
            other => panic!("expected FailClosed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn prefix_reuse_same_turn_reuses_decision() {
        let yaml = r#"
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
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      prefix_reuse: true
      signals:
        keywords:
          - name: needs_explain
            operator: OR
            keywords: ["explain"]
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
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-prefix-1".into());
        let mk = |content: &str| ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!(content),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        // Turn 1: fresh decision, cached under (session, 1).
        let (d1, _, _, _) = route_request(&st, mk("please explain rust"), &meta)
            .await
            .unwrap();
        assert_eq!(d1.model, "local/general");
        assert!(st.decision_cache.get("sess-prefix-1", 1).is_some());
        // Same turn (still 1 user message): served from cache, same model.
        let (d2, _, _, _) = route_request(&st, mk("please explain rust"), &meta)
            .await
            .unwrap();
        assert_eq!(d2.model, d1.model);
        assert!(st.decision_cache.get("sess-prefix-1", 1).is_some());
        // New turn (2 user messages): different key, freshly decided and cached.
        let mut req3 = mk("please explain loops");
        req3.messages.push(aria_router_core::ChatMessage {
            role: "assistant".into(),
            content: json!("ok"),
        });
        req3.messages.push(aria_router_core::ChatMessage {
            role: "user".into(),
            content: json!("follow up"),
        });
        let (d3, _, _, _) = route_request(&st, req3, &meta).await.unwrap();
        assert!(st.decision_cache.get("sess-prefix-1", 2).is_some());
        assert_eq!(d3.model, "local/general");
    }

    #[tokio::test]
    async fn agent_delegation_falls_back_when_endpoint_unreachable() {
        let yaml = r#"
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
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/agent-auto]
    router: agent
    recipe: agent-default
recipes:
  - name: agent-default
    router: agent
    agent:
      timeout_ms: 1000
      max_turns: 3
      fallback: local/general
      delegate_endpoint: http://127.0.0.1:1
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let meta: HashMap<String, String> = HashMap::new();
        let req = ChatRequest {
            model: "ariacompute/agent-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("route this"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        // Unreachable delegate endpoint → decision-ext fallback → bypass(fallback).
        let (d, _, _, _) = route_request(&st, req, &meta).await.unwrap();
        assert_eq!(d.model, "local/general");
        assert_eq!(d.layer, "agent-delegate");
    }

    #[tokio::test]
    async fn prefix_reuse_defaults_on_same_turn() {
        // Omit prefix_reuse → default true; same (session, turn) reuses decision.
        let yaml = r#"
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
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-default-prefix".into());
        let mk = |content: &str| ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!(content),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(&st, mk("please explain rust"), &meta)
            .await
            .unwrap();
        assert!(st.decision_cache.get("sess-default-prefix", 1).is_some());
        let (d2, _, _, _) = route_request(&st, mk("please explain rust"), &meta)
            .await
            .unwrap();
        assert_eq!(d2.model, d1.model);
    }

    #[tokio::test]
    async fn prefix_reuse_false_does_not_cache() {
        let yaml = r#"
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
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      prefix_reuse: false
      signals:
        keywords:
          - name: needs_explain
            operator: OR
            keywords: ["explain"]
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
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-no-prefix".into());
        let req = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let _ = route_request(&st, req, &meta).await.unwrap();
        assert!(st.decision_cache.get("sess-no-prefix", 1).is_none());
    }

    #[tokio::test]
    async fn prefix_reuse_tool_append_holds_model_and_ttl() {
        let yaml = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/fast
  models:
    - name: local/general
      provider_model_id: echo
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/fast
      provider_model_id: echo-fast
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
          emits:
            - kind: retention
              retention:
                ttl_turns: 3
                keep_current_model: true
        - name: default_fast
          priority: 1
          rules:
            operator: AND
            conditions: []
          modelRefs:
            - model: local/fast
global:
  stores:
    memory:
      enabled: true
      default_ttl_turns: 3
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-tool-hold".into());
        let req1 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(&st, req1, &meta).await.unwrap();
        assert_eq!(d1.model, "local/general");
        let ttl_before = st.memory.get("sess-tool-hold").unwrap().ttl_turns_left;
        // Same user turn: append assistant + tool — must freeze model and not tick TTL.
        let req2 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain rust"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("calling tool"),
                },
                aria_router_core::ChatMessage {
                    role: "tool".into(),
                    content: json!("tool result"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d2, _, _, _) = route_request(&st, req2, &meta).await.unwrap();
        assert_eq!(d2.model, "local/general");
        assert_eq!(
            st.memory.get("sess-tool-hold").unwrap().ttl_turns_left,
            ttl_before
        );
    }

    #[tokio::test]
    async fn compaction_clears_sticky_allows_redecide() {
        let yaml = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/fast
  models:
    - name: local/general
      provider_model_id: echo
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/fast
      provider_model_id: echo-fast
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
          emits:
            - kind: retention
              retention:
                ttl_turns: 4
                keep_current_model: true
        - name: default_fast
          priority: 1
          rules:
            operator: AND
            conditions: []
          modelRefs:
            - model: local/fast
global:
  stores:
    memory:
      enabled: true
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-compact".into());
        let req1 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "system".into(),
                    content: json!("sys-v1"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain rust"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(&st, req1, &meta).await.unwrap();
        assert_eq!(d1.model, "local/general");
        assert!(st.memory.get("sess-compact").is_some());
        // Compaction: fewer messages + new system prefix → clear sticky.
        let req2 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "system".into(),
                    content: json!("sys-v2-summary"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("hi"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d2, _, _, _) = route_request(&st, req2, &meta).await.unwrap();
        assert_eq!(d2.model, "local/fast");
    }

    #[tokio::test]
    async fn afm_d_sticky_holds_across_user_turns() {
        let yaml = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/small
  models:
    - name: local/small
      tier: small
      provider_model_id: echo-s
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/mid
      tier: mid
      provider_model_id: echo-m
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/large
      tier: large
      provider_model_id: echo-l
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default
recipes:
  - name: afm-d-default
    router: afm-d
    afm-d:
      timeout_ms: 1000
      fallback: local/mid
global:
  stores:
    memory:
      enabled: true
      default_ttl_turns: 4
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-afmd-sticky".into());
        let req1 = ChatRequest {
            model: "ariacompute/afm-d-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain how rust ownership works"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(&st, req1, &meta).await.unwrap();
        assert_eq!(d1.model, "local/large");
        assert_eq!(d1.retention_keep_current_model, Some(true));
        // New user turn with greeting: without sticky would shortcut to small.
        let req2 = ChatRequest {
            model: "ariacompute/afm-d-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain how rust ownership works"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("..."),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("hi"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d2, _, _, _) = route_request(&st, req2, &meta).await.unwrap();
        assert_eq!(d2.model, "local/large");
        assert!(d2.reason.starts_with("sticky:"));
        // Compaction clears sticky → greeting routes to small.
        let req3 = ChatRequest {
            model: "ariacompute/afm-d-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("hi"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d3, _, _, _) = route_request(&st, req3, &meta).await.unwrap();
        assert_eq!(d3.model, "local/small");
    }

    #[tokio::test]
    async fn agent_sticky_holds_across_user_turns() {
        let yaml = r#"
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
          endpoint: 127.0.0.1:9
    - name: local/other
      provider_model_id: echo-o
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/agent-auto]
    router: agent
    recipe: agent-default
recipes:
  - name: agent-default
    router: agent
    agent:
      timeout_ms: 1000
      max_turns: 3
      fallback: local/general
      delegate_endpoint: http://127.0.0.1:1
global:
  stores:
    memory:
      enabled: true
      default_ttl_turns: 4
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-agent-sticky".into());
        let req1 = ChatRequest {
            model: "ariacompute/agent-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("route this"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(&st, req1, &meta).await.unwrap();
        assert_eq!(d1.model, "local/general");
        assert_eq!(d1.retention_keep_current_model, Some(true));
        let req2 = ChatRequest {
            model: "ariacompute/agent-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("route this"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("ok"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("again please"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d2, _, _, _) = route_request(&st, req2, &meta).await.unwrap();
        assert_eq!(d2.model, "local/general");
        assert!(d2.reason.starts_with("sticky:"));
    }

    #[tokio::test]
    async fn no_explicit_session_skips_prefix_reuse_cache() {
        // Without metadata session, DecisionCache must not freeze across calls.
        let yaml = r#"
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
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let meta = HashMap::new();
        let req = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let _ = route_request(&st, req, &meta).await.unwrap();
        assert!(st.decision_cache.get("", 1).is_none());
        // Ephemeral keys are never written under an empty session.
        assert!(st.decision_cache.get("sess-none", 1).is_none());
    }

    #[tokio::test]
    async fn body_session_enables_prefix_reuse_via_http() {
        let backend = mock_upstream().await;
        let host = backend.trim_start_matches("http://");
        let yaml = format!(
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
      backend_refs:
        - name: primary
          endpoint: {host}
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
global:
  require_api_key: false
"#
        );
        let doc = RouterDocument::from_yaml_str(&yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "session": "body-sess-1",
            "messages": [{"role":"user","content":"please explain rust"}]
        })
        .to_string();
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(st.decision_cache.get("body-sess-1", 1).is_some());
    }

    #[tokio::test]
    async fn tools_fingerprint_change_compacts_decision_cache() {
        let yaml = r#"
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
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-tools-fp".into());
        let mut extra = serde_json::Map::new();
        extra.insert("tools".into(), json!([{"name":"search"}]));
        let req1 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: extra.clone(),
        };
        let _ = route_request(&st, req1, &meta).await.unwrap();
        assert!(st.decision_cache.get("sess-tools-fp", 1).is_some());
        // Same user turn / messages, but tools payload changed → cache busted.
        extra.insert("tools".into(), json!([{"name":"browse"}]));
        let req2 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra,
        };
        let _ = route_request(&st, req2, &meta).await.unwrap();
        // After compaction the prior turn-1 entry is cleared, then re-put for the new baseline.
        assert!(st.decision_cache.get("sess-tools-fp", 1).is_some());
        // Changing tools alone must have gone through observe_and_check_compact (compacted=true).
        // Prove isolation: a third call with the *same* tools hits the frozen decision.
        let mut extra3 = serde_json::Map::new();
        extra3.insert("tools".into(), json!([{"name":"browse"}]));
        let req3 = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: extra3,
        };
        let (d3, _, _, _) = route_request(&st, req3, &meta).await.unwrap();
        assert_eq!(d3.model, "local/general");
    }

    #[tokio::test]
    async fn sticky_ttl_expires_after_user_turns() {
        let yaml = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/fast
  models:
    - name: local/general
      provider_model_id: echo
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/fast
      provider_model_id: echo-fast
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
          emits:
            - kind: retention
              retention:
                ttl_turns: 2
                keep_current_model: true
        - name: default_fast
          priority: 1
          rules:
            operator: AND
            conditions: []
          modelRefs:
            - model: local/fast
global:
  stores:
    memory:
      enabled: true
      default_ttl_turns: 2
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-ttl-exp".into());
        let mk = |msgs: Vec<aria_router_core::ChatMessage>| ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: msgs,
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d1, _, _, _) = route_request(
            &st,
            mk(vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain rust"),
            }]),
            &meta,
        )
        .await
        .unwrap();
        assert_eq!(d1.model, "local/general");
        assert_eq!(st.memory.get("sess-ttl-exp").unwrap().ttl_turns_left, 2);
        // Turn 2: TTL ticks to 1; sticky still holds for a non-keyword follow-up.
        let (d2, _, _, _) = route_request(
            &st,
            mk(vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain rust"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("ok"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("thanks"),
                },
            ]),
            &meta,
        )
        .await
        .unwrap();
        assert_eq!(d2.model, "local/general");
        assert_eq!(st.memory.get("sess-ttl-exp").unwrap().ttl_turns_left, 1);
        // Turn 3: TTL expires → sticky gone → default_fast.
        let (d3, _, _, _) = route_request(
            &st,
            mk(vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain rust"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("ok"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("thanks"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("np"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("bye"),
                },
            ]),
            &meta,
        )
        .await
        .unwrap();
        assert_eq!(d3.model, "local/fast");
        assert!(st.memory.get("sess-ttl-exp").is_none());
    }

    #[tokio::test]
    async fn afm_d_ineligible_sticky_falls_through_to_route() {
        let yaml = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/small
  models:
    - name: local/small
      tier: small
      provider_model_id: echo-s
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/large
      tier: large
      provider_model_id: echo-l
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default
recipes:
  - name: afm-d-default
    router: afm-d
    afm-d:
      timeout_ms: 1000
      fallback: local/small
global:
  stores:
    memory:
      enabled: true
      default_ttl_turns: 4
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc.clone());
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-afmd-badsticky".into());
        // Pre-seed sticky with a model that will fail hard_filter.
        st.memory.pin(
            "sess-afmd-badsticky",
            "gone/missing".into(),
            "prior".into(),
            4,
            1,
            &doc.global.stores.memory,
        );
        let req = ChatRequest {
            model: "ariacompute/afm-d-auto".into(),
            messages: vec![
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("please explain how rust ownership works"),
                },
                aria_router_core::ChatMessage {
                    role: "assistant".into(),
                    content: json!("…"),
                },
                aria_router_core::ChatMessage {
                    role: "user".into(),
                    content: json!("hi"),
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        // Turn 2: sticky ineligible → fall through; greeting shortcuts to small.
        let (d, _, _, _) = route_request(&st, req, &meta).await.unwrap();
        assert_eq!(d.model, "local/small");
        assert!(!d.reason.starts_with("sticky:"));
    }

    #[tokio::test]
    async fn afm_d_without_memory_does_not_pin_sticky() {
        let yaml = r#"
version: v0.3
listeners:
  - name: http
    address: 127.0.0.1
    port: 8899
providers:
  defaults:
    default_model: local/small
  models:
    - name: local/small
      tier: small
      provider_model_id: echo-s
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
    - name: local/large
      tier: large
      provider_model_id: echo-l
      locality: local
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default
recipes:
  - name: afm-d-default
    router: afm-d
    afm-d:
      timeout_ms: 1000
      fallback: local/small
global:
  stores:
    memory:
      enabled: false
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-afmd-nomem".into());
        let req = ChatRequest {
            model: "ariacompute/afm-d-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("please explain how rust ownership works"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d, _, _, _) = route_request(&st, req, &meta).await.unwrap();
        assert_eq!(d.model, "local/large");
        assert!(d.retention_keep_current_model.is_none());
        assert!(st.memory.get("sess-afmd-nomem").is_none());
        // DecisionCache still freezes the same turn when prefix_reuse defaults on.
        assert!(st.decision_cache.get("sess-afmd-nomem", 1).is_some());
    }

    #[tokio::test]
    async fn agent_prefix_reuse_false_skips_sticky_pin() {
        let yaml = r#"
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
          endpoint: 127.0.0.1:9
entrypoints:
  - model_names: [ariacompute/agent-auto]
    router: agent
    recipe: agent-default
recipes:
  - name: agent-default
    router: agent
    agent:
      prefix_reuse: false
      timeout_ms: 1000
      max_turns: 3
      fallback: local/general
      delegate_endpoint: http://127.0.0.1:1
global:
  stores:
    memory:
      enabled: true
      default_ttl_turns: 4
"#;
        let doc = RouterDocument::from_yaml_str(yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let mut meta = HashMap::new();
        meta.insert("session".into(), "sess-agent-noprefix".into());
        let req = ChatRequest {
            model: "ariacompute/agent-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: json!("route this"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let (d, _, _, _) = route_request(&st, req, &meta).await.unwrap();
        assert_eq!(d.model, "local/general");
        assert!(st.decision_cache.get("sess-agent-noprefix", 1).is_none());
        assert!(st.memory.get("sess-agent-noprefix").is_none());
        assert!(d.retention_keep_current_model.is_none());
    }

    #[tokio::test]
    async fn x_aria_session_header_enables_context_hold() {
        let backend = mock_upstream().await;
        let host = backend.trim_start_matches("http://");
        let yaml = format!(
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
      backend_refs:
        - name: primary
          endpoint: {host}
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
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
global:
  require_api_key: false
"#
        );
        let doc = RouterDocument::from_yaml_str(&yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [{"role":"user","content":"please explain rust"}]
        })
        .to_string();
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-aria-session", "hdr-sess-1")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(st.decision_cache.get("hdr-sess-1", 1).is_some());
    }

    #[tokio::test]
    async fn soft_ratelimit_429() {
        let backend = mock_upstream().await;
        let mut doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        doc.global.services.ratelimit.enabled = true;
        doc.global.services.ratelimit.requests_per_minute = 1;
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st);
        let mk = || {
            Request::post("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "ariacompute/semantic-auto",
                        "messages": [{"role":"user","content":"explain x"}]
                    })
                    .to_string(),
                ))
                .unwrap()
        };
        let res1 = app.clone().oneshot(mk()).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        let res2 = app.oneshot(mk()).await.unwrap();
        assert_eq!(res2.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn semantic_cache_hit_skips_upstream() {
        let backend = mock_upstream().await;
        let host = backend.trim_start_matches("http://");
        let yaml = include_str!("../../config/examples/semantic-stateful.yaml")
            .replace("127.0.0.1:8000", host)
            .replace("127.0.0.1:8001", host);
        let doc = RouterDocument::from_yaml_str(&yaml).unwrap();
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
        let body = json!({
            "model": "ariacompute/semantic-auto",
            "messages": [{"role":"user","content":"please explain rust"}]
        })
        .to_string();
        let mk = || {
            Request::post("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-aria-session", "cache-sess-1")
                .body(Body::from(body.clone()))
                .unwrap()
        };
        let res1 = app.clone().oneshot(mk()).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        assert!(res1.headers().get("x-aria-router-semantic-cache").is_none());

        let res2 = app.oneshot(mk()).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);
        assert_eq!(
            res2.headers()
                .get("x-aria-router-semantic-cache")
                .and_then(|v| v.to_str().ok()),
            Some("hit")
        );
        let last = st.last_route.lock().unwrap().clone().unwrap();
        assert_eq!(last.semantic_cache_hit, Some(true));
    }

    #[tokio::test]
    async fn replay_log_lists_recent() {
        let backend = mock_upstream().await;
        let mut doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        doc.global.services.router_replay.enabled = true;
        doc.global.services.router_replay.max_items = 16;
        let (st, _dir) = isolated_state(doc);
        let app = data_router(st.clone());
        let res = app
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "model": "ariacompute/semantic-auto",
                            "messages": [{"role":"user","content":"explain replay"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let recent = st.replay_log.recent(5);
        assert!(!recent.is_empty());
        assert!(!recent[0].id.is_empty());
        assert_eq!(st.replay_log.get(&recent[0].id).unwrap().id, recent[0].id);
    }

    #[tokio::test]
    async fn semantic_stateful_yaml_validates() {
        let raw = include_str!("../../config/examples/semantic-stateful.yaml");
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        assert!(doc.global.stores.memory.enabled);
        assert!(doc.global.stores.semantic_cache.enabled);
    }
}
