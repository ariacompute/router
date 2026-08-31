//! OpenAI data plane + management API + routing pipeline.

use aria_router_agent::{task_from, AgentExtension, BuiltinExtension, FakeExtension};
use aria_router_algorithm::{hard_filter, select, RuntimeStats};
use aria_router_config::{ExtensionCfg, Recipe, RouterDocument};
use aria_router_core::{
    ChatRequest, RouteDecision, RouterError, RouterKind,
};
use aria_router_decision::select_decision;
use aria_router_ext::SubprocessExtension;
use aria_router_plugin::{apply_request, extra_headers, remember_response, PluginHost, PluginOutcome};
use aria_router_provider::{forward, forward_sse_text, PoolState};
use aria_router_signal::extract;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub doc: Mutex<RouterDocument>,
    pub pool: PoolState,
    pub plugins: PluginHost,
    pub last_route: Mutex<Option<RouteDecision>>,
    pub replay: Mutex<Vec<RouteDecision>>,
    pub fake_agents: Mutex<HashMap<String, RouteDecision>>,
}

impl AppState {
    pub fn new(doc: RouterDocument) -> Self {
        Self {
            doc: Mutex::new(doc),
            pool: PoolState::default(),
            plugins: PluginHost::default(),
            last_route: Mutex::new(None),
            replay: Mutex::new(Vec::new()),
            fake_agents: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_fake_agent(&self, name: &str, d: RouteDecision) {
        self.fake_agents.lock().unwrap().insert(name.to_string(), d);
    }
}

pub fn data_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat))
        .with_state(state)
}

pub fn mgmt_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/router/validate", post(validate_ep))
        .route("/v1/router/replay", get(replay_ep))
        .route("/v1/router/providers", put(upsert_provider))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
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
    Json(body): Json<ProviderUpsert>,
) -> Result<Json<Value>, AppError> {
    apply_upsert(&st, body).map_err(AppError)
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

async fn chat(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let req: ChatRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return AppError(RouterError::InvalidParam(e.to_string())).into_response();
        }
    };
    let metadata = metadata_from_headers(&headers);
    let want_stream = req.stream;
    match route_and_forward(st, req, want_stream, metadata).await {
        Ok(r) => r,
        Err(e) => AppError(e).into_response(),
    }
}

async fn route_and_forward(
    st: Arc<AppState>,
    req: ChatRequest,
    want_stream: bool,
    metadata: HashMap<String, String>,
) -> Result<Response, RouterError> {
    let (decision, mut fwd, extra, hdrs) = route_request(&st, req, &metadata).await?;
    record(&st, decision.clone());
    match extra {
        Some(fast) => {
            let mut res = Json(fast).into_response();
            attach_route_headers(res.headers_mut(), &decision);
            Ok(res)
        }
        None => {
            let doc = snapshot_doc(&st);
            if want_stream {
                fwd.stream = true;
                let text = forward_sse_text(&doc, &decision.model, &fwd, &hdrs, &st.pool).await?;
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
                let mut res = Json(body).into_response();
                attach_route_headers(res.headers_mut(), &decision);
                Ok(res)
            }
        }
    }
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
    let stats = RuntimeStats {
        latency_ms: st.pool.latency_map(),
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
    let ext_cfg = doc
        .extensions
        .iter()
        .find(|e| e.name == agent.extension)
        .cloned()
        .ok_or_else(|| RouterError::Config("extension not found".into()))?;
    let all_names: Vec<String> = doc.providers.models.iter().map(|m| m.name.clone()).collect();
    let eligible = hard_filter(doc, &all_names, Some("local"), Some("text"));
    if eligible.is_empty() {
        return Err(RouterError::FailClosed("no eligible models after hard constraints".into()));
    }
    let task = task_from(&req, eligible.clone(), agent, &ext_cfg);
    let mut decision = invoke_extension(st, &ext_cfg, agent, task).await?;
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

async fn invoke_extension(
    st: &AppState,
    ext: &ExtensionCfg,
    agent: &aria_router_config::AgentRecipe,
    task: aria_router_agent::RouteTask,
) -> Result<RouteDecision, RouterError> {
    let canned = st.fake_agents.lock().unwrap().get(&ext.name).cloned();
    if let Some(fake) = canned {
        let f = FakeExtension {
            name: ext.name.clone(),
            decision: Ok(fake),
        };
        return f.route(task).await;
    }
    match ext.ext_type.as_str() {
        "builtin" => {
            let b = BuiltinExtension {
                endpoint: agent.endpoint.clone().or(ext.endpoint.clone()),
                model: agent.model.clone().unwrap_or_else(|| "router-llm".into()),
                canned: None,
            };
            b.route(task).await
        }
        "pi" | "deepseek-harness" => {
            let sub = SubprocessExtension { cfg: ext.clone() };
            sub.ensure_binary()?;
            sub.route(task).await
        }
        other => Err(RouterError::Unsupported(format!("extension type {other}"))),
    }
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
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

pub fn ensure_extensions_startable(doc: &RouterDocument) -> Result<(), RouterError> {
    for ext in &doc.extensions {
        match ext.ext_type.as_str() {
            "builtin" => {}
            "pi" | "deepseek-harness" => {
                SubprocessExtension { cfg: ext.clone() }.ensure_binary()?;
            }
            other => return Err(RouterError::Config(format!("unknown extension type {other}"))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_config::ExtensionCfg;
    use aria_router_ext::SubprocessExtension;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

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
extensions:
  - name: builtin
    type: builtin
entrypoints:
  - model_names: [aria/semantic-auto]
    router: semantic
    recipe: mom
  - model_names: [aria/agent-auto]
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
      extension: builtin
      fallback: local/general
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
                }]
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
        let st = Arc::new(AppState::new(doc));
        let app = data_router(st);
        let body = json!({
            "model": "aria/semantic-auto",
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
        let st = Arc::new(AppState::new(doc));
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
        let st = Arc::new(AppState::new(doc));
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
            "model": "aria/agent-auto",
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
            "model": "aria/semantic-auto",
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
        let st = Arc::new(AppState::new(doc));
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
            "model": "aria/agent-auto",
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
        let st = Arc::new(AppState::new(doc));
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
    async fn sse_and_upsert_and_missing_ext() {
        let backend = mock_upstream().await;
        let doc = RouterDocument::from_yaml_str(&tiny_yaml(&backend)).unwrap();
        let st = Arc::new(AppState::new(doc));
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

        let mut missing = ExtensionCfg {
            name: "pi".into(),
            ext_type: "pi".into(),
            command: vec!["aria-router-pi-not-installed".into()],
            workdir: None,
            timeout_ms: Some(10),
            env: Default::default(),
            endpoint: None,
        };
        let err = SubprocessExtension { cfg: missing.clone() }
            .ensure_binary()
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
        missing.ext_type = "deepseek-harness".into();
        missing.command = vec!["aria-router-dsh-not-installed".into()];
        assert!(SubprocessExtension { cfg: missing }.ensure_binary().is_err());
    }
}
