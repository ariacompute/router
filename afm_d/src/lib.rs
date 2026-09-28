//! AFM-D System One decisioner: eligible models → Choice → RouteDecision.

use aria_router_config::AfmDRecipe;
use aria_router_core::{ChatRequest, ModelCard, RouteDecision, RouterError};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const MAX_CHOICE_OPTIONS: usize = 255;
const DEFAULT_INSTRUCTIONS: &str = "\
Pick the cheapest sufficient model tier for the user message.
small = greetings/thanks/short chitchat/factoids/acronyms
mid = systems judgment and trade-offs
large = teaching and how-it-works explanations
Greetings (hi/hello/hey) are always small.";

/// Exact phrases that must route to the small tier without calling System One.
/// Encoder Choice is unreliable on bare greetings when criteria also list "large".
fn is_trivial_chitchat(text: &str) -> bool {
    let mut cleaned = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphanumeric() || c.is_whitespace() {
            cleaned.push(c.to_ascii_lowercase());
        } else if c == '\'' {
            // drop apostrophes so "what's" → "whats" still won't match greetings
        } else {
            cleaned.push(' ');
        }
    }
    let t = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        t.as_str(),
        "hi" | "hello" | "hey" | "hiya" | "yo" | "sup" | "howdy"
            | "thanks" | "thank you" | "thx" | "ty"
            | "good morning" | "good afternoon" | "good evening" | "good night"
            | "bye" | "goodbye" | "see you" | "ok" | "okay" | "yes" | "no" | "yep" | "nope"
    )
}

fn tier_of(card: &ModelCard) -> &'static str {
    if let Some(t) = card.tier.as_deref() {
        match t.trim().to_ascii_lowercase().as_str() {
            "small" => return "small",
            "mid" | "medium" => return "mid",
            "large" => return "large",
            _ => {}
        }
    }
    let lower = card.name.to_ascii_lowercase();
    if lower.contains("ariamodel-large") || lower.ends_with("-large") || lower.ends_with("/large") {
        "large"
    } else if lower.contains("ariamodel-mid") || lower.ends_with("-mid") || lower.ends_with("/mid") {
        "mid"
    } else if lower.contains("ariamodel-small")
        || lower.ends_with("-small")
        || lower.ends_with("/small")
    {
        "small"
    } else {
        "unknown"
    }
}

fn pick_tier<'a>(eligible: &'a [ModelCard], tier: &str) -> Option<&'a ModelCard> {
    eligible.iter().find(|m| tier_of(m) == tier)
}

#[derive(Debug, Clone)]
pub struct AfmDTask {
    pub state: Value,
    pub eligible: Vec<ModelCard>,
    pub instructions: String,
    pub descriptions: HashMap<String, String>,
    pub timeout_ms: u64,
    pub min_confidence: Option<f32>,
    pub fallback: Option<String>,
}

impl AfmDTask {
    /// Best-effort user utterance for shortcuts / logging.
    fn state_user_text(&self) -> Option<String> {
        if let Some(s) = self.state.as_str() {
            let s = s.trim();
            let stripped = s
                .strip_prefix("User message:")
                .or_else(|| s.strip_prefix("user message:"))
                .unwrap_or(s)
                .trim();
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
        self.state
            .get("last_user")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
}

/// HTTP client for aria-engine `POST /v1/systemone`.
pub struct AfmDDecisioner {
    pub endpoint: Option<String>,
}

impl AfmDDecisioner {
    pub async fn route(&self, task: AfmDTask) -> Result<RouteDecision, RouterError> {
        if task.eligible.is_empty() {
            return Err(RouterError::FailClosed("no eligible models".into()));
        }
        if task.eligible.len() > MAX_CHOICE_OPTIONS {
            return Err(RouterError::FailClosed(format!(
                "afm-d eligible pool size {} exceeds Choice max {MAX_CHOICE_OPTIONS}",
                task.eligible.len()
            )));
        }
        // Deterministic small-tier path for bare greetings / chitchat — System One
        // Choice otherwise skews to large when "teaching" is in the option set.
        if let Some(text) = task.state_user_text() {
            if is_trivial_chitchat(&text) {
                if let Some(m) = pick_tier(&task.eligible, "small") {
                    return Ok(tier_decision(m, "afm-d:chitchat-small"));
                }
            }
        }
        // No endpoint (missing / blank), or a single eligible model: skip System One.
        let endpoint = self
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(normalize_endpoint);
        if endpoint.is_none() || task.eligible.len() == 1 {
            return Ok(first_eligible(&task.eligible));
        }

        let endpoint = endpoint.unwrap();
        let body = build_systemone_body(&task);
        let url = format!("{}/v1/systemone", endpoint.trim_end_matches('/'));
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return apply_fallback_or_err(
                &task,
                RouterError::Config(format!(
                    "afm-d endpoint must be an absolute http(s) URL, got {endpoint:?}"
                )),
            );
        }
        static AFM_HTTP: OnceLock<reqwest::Client> = OnceLock::new();
        let client = AFM_HTTP.get_or_init(reqwest::Client::new);
        let timeout = std::time::Duration::from_millis(task.timeout_ms.max(1));
        let resp = tokio::time::timeout(timeout, client.post(&url).json(&body).send())
            .await
            .map_err(|_| RouterError::Timeout("afm-d systemone".into()))?
            .map_err(|e| RouterError::Extension(format_reqwest("afm-d systemone", &e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return apply_fallback_or_err(
                &task,
                RouterError::Extension(format!("afm-d systemone {status}: {text}")),
            );
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| RouterError::Extension(format_reqwest("afm-d systemone decode", &e)))?;
        match decision_from_systemone(&v, &task) {
            Ok(d) => Ok(d),
            Err(e) => apply_fallback_or_err(&task, e),
        }
    }
}

fn format_reqwest(ctx: &str, e: &reqwest::Error) -> String {
    // reqwest's Display for builder errors is just "builder error"; include source chain.
    let mut msg = format!("{ctx}: {e}");
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    if let Some(url) = e.url() {
        msg.push_str(" (");
        msg.push_str(url.as_str());
        msg.push(')');
    }
    msg
}

/// Ensure System One base has an http(s) scheme. Bare `host:port` is common when
/// env vars omit the scheme and reqwest rejects them as relative URLs.
fn normalize_endpoint(raw: &str) -> String {
    let ep = raw.trim().trim_end_matches('/');
    if ep.starts_with("http://") || ep.starts_with("https://") {
        return ep.to_string();
    }
    format!("http://{ep}")
}

fn first_eligible(eligible: &[ModelCard]) -> RouteDecision {
    tier_decision(&eligible[0], "afm-d:first-eligible")
}

fn tier_decision(model: &ModelCard, reason: &str) -> RouteDecision {
    RouteDecision {
        model: model.name.clone(),
        algorithm: Some("static".into()),
        reason: reason.into(),
        confidence: 0.5,
        layer: "afm-d".into(),
        decision: "afm-d".into(),
        bypass: false,
        routing_latency_ms: None,
        cache_response: false,
        retention_drop: None,
        retention_ttl_turns: None,
        retention_keep_current_model: None,
        retention_prefer_prefix: None,
        semantic_cache_hit: None,
        replay_id: None,
        session: None,
    }
}

fn apply_fallback_or_err(
    task: &AfmDTask,
    err: RouterError,
) -> Result<RouteDecision, RouterError> {
    if let Some(fb) = &task.fallback {
        if task.eligible.iter().any(|m| m.name == *fb) {
            return Ok(RouteDecision {
                model: fb.clone(),
                algorithm: Some("static".into()),
                reason: format!("afm-d:fallback:{fb}"),
                confidence: 0.0,
                layer: "afm-d".into(),
                decision: "afm-d".into(),
                bypass: false,
                routing_latency_ms: None,
                cache_response: false,
                retention_drop: None,
                retention_ttl_turns: None,
                retention_keep_current_model: None,
                retention_prefer_prefix: None,
                semantic_cache_hit: None,
                replay_id: None,
                session: None,
            });
        }
    }
    Err(err)
}

pub fn build_systemone_body(task: &AfmDTask) -> Value {
    let mut criteria = Map::new();
    for m in &task.eligible {
        let desc = task
            .descriptions
            .get(&m.name)
            .cloned()
            .unwrap_or_else(|| synthesize_description(m));
        criteria.insert(m.name.clone(), Value::String(desc));
    }
    json!({
        "state": task.state,
        "questions": {
            "route": {
                "type": "choice",
                "instructions": task.instructions,
                "criteria": criteria
            }
        }
    })
}

pub fn synthesize_description(m: &ModelCard) -> String {
    let tier = m
        .tier
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("unknown");
    let caps = if m.capabilities.is_empty() {
        "chat".into()
    } else {
        m.capabilities.join(",")
    };
    format!(
        "tier={tier}; locality={}; modality={}; capabilities={caps}",
        m.locality, m.modality
    )
}

pub fn decision_from_systemone(
    resp: &Value,
    task: &AfmDTask,
) -> Result<RouteDecision, RouterError> {
    let answer = resp
        .pointer("/answers/route")
        .ok_or_else(|| RouterError::FailClosed("afm-d missing answers.route".into()))?;
    let choice = answer
        .get("choice")
        .and_then(|c| c.as_str())
        .ok_or_else(|| RouterError::FailClosed("afm-d missing answers.route.choice".into()))?
        .to_string();
    if !task.eligible.iter().any(|m| m.name == choice) {
        return Err(RouterError::FailClosed(format!(
            "afm-d chose {choice} not in eligible pool"
        )));
    }
    let confidence = answer
        .get("confidence")
        .and_then(|c| c.as_f64())
        .map(|c| c as f32)
        .unwrap_or(1.0);
    if let Some(min_c) = task.min_confidence {
        if confidence < min_c {
            return Err(RouterError::FailClosed(format!(
                "afm-d confidence {confidence} below min_confidence {min_c}"
            )));
        }
    }
    Ok(RouteDecision {
        model: choice.clone(),
        algorithm: Some("static".into()),
        reason: format!("afm-d:{choice}"),
        confidence,
        layer: "afm-d".into(),
        decision: "afm-d".into(),
        bypass: false,
        routing_latency_ms: None,
        cache_response: false,
        retention_drop: None,
        retention_ttl_turns: None,
        retention_keep_current_model: None,
        retention_prefer_prefix: None,
        semantic_cache_hit: None,
        replay_id: None,
        session: None,
    })
}

pub fn task_from(req: &ChatRequest, eligible: Vec<ModelCard>, recipe: &AfmDRecipe) -> AfmDTask {
    // Plain "User message: …" packs better for Encoder Choice than a JSON blob;
    // bare greetings otherwise skew toward the large/"teaching" option.
    let last = req.last_user_text();
    let state = if last.trim().is_empty() {
        Value::String(req.prompt_text())
    } else {
        Value::String(format!("User message: {}", last.trim()))
    };
    AfmDTask {
        state,
        eligible,
        instructions: recipe
            .instructions
            .clone()
            .unwrap_or_else(|| DEFAULT_INSTRUCTIONS.into()),
        descriptions: recipe.descriptions.clone(),
        timeout_ms: recipe.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
        min_confidence: recipe.min_confidence,
        fallback: recipe.fallback.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_core::ChatMessage;
    use axum::routing::post;
    use axum::{Json, Router};

    fn card(name: &str) -> ModelCard {
        ModelCard {
            name: name.into(),
            locality: "local".into(),
            modality: "text".into(),
            capabilities: vec!["chat".into()],
            provider_model_id: "x".into(),
            tier: None,
        }
    }

    fn card_tier(name: &str, tier: &str) -> ModelCard {
        ModelCard {
            name: name.into(),
            locality: "cloud".into(),
            modality: "text".into(),
            capabilities: vec!["chat".into()],
            provider_model_id: "x".into(),
            tier: Some(tier.into()),
        }
    }

    fn base_task(eligible: Vec<ModelCard>) -> AfmDTask {
        AfmDTask {
            state: json!("hello"),
            eligible,
            instructions: DEFAULT_INSTRUCTIONS.into(),
            descriptions: HashMap::new(),
            timeout_ms: 2000,
            min_confidence: None,
            fallback: None,
        }
    }

    #[test]
    fn normalize_endpoint_adds_http_scheme() {
        assert_eq!(
            normalize_endpoint("127.0.0.1:8011"),
            "http://127.0.0.1:8011"
        );
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:8011/"),
            "http://127.0.0.1:8011"
        );
        assert_eq!(
            normalize_endpoint("https://engine.example"),
            "https://engine.example"
        );
    }

    #[tokio::test]
    async fn host_port_without_scheme_reaches_http() {
        let app = Router::new().route(
            "/v1/systemone",
            post(|| async {
                Json(json!({
                    "answers": {
                        "route": {
                            "choice": "local/b",
                            "confidence": 0.91
                        }
                    }
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Bare host:port (no scheme) must not yield reqwest "relative URL" builder error.
        let d = AfmDDecisioner {
            endpoint: Some(addr.to_string()),
        }
        .route(base_task(vec![card("local/a"), card("local/b")]))
        .await
        .unwrap();
        assert_eq!(d.model, "local/b");
    }

    #[tokio::test]
    async fn blank_endpoint_skips_http() {
        let d = AfmDDecisioner {
            endpoint: Some("  ".into()),
        }
        .route(base_task(vec![card("local/a"), card("local/b")]))
        .await
        .unwrap();
        assert_eq!(d.model, "local/a");
        assert_eq!(d.reason, "afm-d:first-eligible");
    }

    #[tokio::test]
    async fn first_eligible_without_endpoint() {
        let d = AfmDDecisioner { endpoint: None }
            .route(base_task(vec![card("local/a"), card("local/b")]))
            .await
            .unwrap();
        assert_eq!(d.model, "local/a");
        assert_eq!(d.reason, "afm-d:first-eligible");
        assert_eq!(d.layer, "afm-d");
    }

    #[tokio::test]
    async fn single_eligible_skips_http() {
        let d = AfmDDecisioner {
            endpoint: Some("http://127.0.0.1:1".into()),
        }
        .route(base_task(vec![card("local/only")]))
        .await
        .unwrap();
        assert_eq!(d.model, "local/only");
    }

    #[test]
    fn body_uses_descriptions_and_synth() {
        let mut descs = HashMap::new();
        descs.insert("a/small".into(), "small factoids".into());
        let task = AfmDTask {
            descriptions: descs,
            ..base_task(vec![card_tier("a/small", "small"), card_tier("a/large", "large")])
        };
        let body = build_systemone_body(&task);
        assert_eq!(body["questions"]["route"]["type"], "choice");
        assert_eq!(body["questions"]["route"]["criteria"]["a/small"], "small factoids");
        let large = body["questions"]["route"]["criteria"]["a/large"]
            .as_str()
            .unwrap();
        assert!(large.contains("tier=large"));
    }

    #[test]
    fn parse_choice_and_reject_unknown() {
        let task = base_task(vec![card("local/general")]);
        let ok = decision_from_systemone(
            &json!({
                "answers": {
                    "route": {
                        "choice": "local/general",
                        "confidence": 0.88,
                        "probabilities": {"local/general": 0.88}
                    }
                }
            }),
            &task,
        )
        .unwrap();
        assert_eq!(ok.model, "local/general");
        assert!((ok.confidence - 0.88).abs() < 1e-5);
        assert_eq!(ok.reason, "afm-d:local/general");

        assert!(decision_from_systemone(
            &json!({"answers": {"route": {"choice": "cloud/x", "confidence": 1.0}}}),
            &task,
        )
        .is_err());
    }

    #[test]
    fn min_confidence_rejects() {
        let mut task = base_task(vec![card("local/general")]);
        task.min_confidence = Some(0.9);
        let err = decision_from_systemone(
            &json!({"answers": {"route": {"choice": "local/general", "confidence": 0.5}}}),
            &task,
        )
        .unwrap_err();
        assert!(matches!(err, RouterError::FailClosed(_)));
    }

    #[tokio::test]
    async fn fallback_on_low_confidence() {
        let app = Router::new().route(
            "/v1/systemone",
            post(|| async {
                Json(json!({
                    "answers": {
                        "route": {"choice": "local/a", "confidence": 0.1}
                    }
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut task = base_task(vec![card("local/a"), card("local/b")]);
        task.min_confidence = Some(0.5);
        task.fallback = Some("local/b".into());
        let d = AfmDDecisioner {
            endpoint: Some(format!("http://{addr}")),
        }
        .route(task)
        .await
        .unwrap();
        assert_eq!(d.model, "local/b");
        assert!(d.reason.contains("fallback"));
    }

    #[tokio::test]
    async fn http_choice_ok() {
        let app = Router::new().route(
            "/v1/systemone",
            post(|| async {
                Json(json!({
                    "answers": {
                        "route": {
                            "choice": "local/b",
                            "confidence": 0.91,
                            "probabilities": {"local/a": 0.09, "local/b": 0.91}
                        }
                    },
                    "model": "afm-dd"
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let d = AfmDDecisioner {
            endpoint: Some(format!("http://{addr}")),
        }
        .route(base_task(vec![card("local/a"), card("local/b")]))
        .await
        .unwrap();
        assert_eq!(d.model, "local/b");
        assert!((d.confidence - 0.91).abs() < 1e-5);
    }

    #[test]
    fn task_from_builds_state() {
        let recipe = AfmDRecipe {
            endpoint: None,
            timeout_ms: Some(1234),
            fallback: Some("local/general".into()),
            instructions: Some("pick".into()),
            descriptions: HashMap::new(),
            min_confidence: Some(0.2),
        };
        let req = ChatRequest {
            model: "ariacompute/afm-d-auto".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: json!("hi there"),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let t = task_from(&req, vec![card("local/general")], &recipe);
        assert_eq!(t.timeout_ms, 1234);
        assert_eq!(t.instructions, "pick");
        assert_eq!(t.state, "User message: hi there");
    }

    #[test]
    fn trivial_chitchat_detects_greetings() {
        assert!(is_trivial_chitchat("hi"));
        assert!(is_trivial_chitchat("Hi!"));
        assert!(is_trivial_chitchat("  HELLO  "));
        assert!(is_trivial_chitchat("thank you"));
        assert!(!is_trivial_chitchat("hi, explain rust ownership"));
        assert!(!is_trivial_chitchat("what is HTTP?"));
    }

    #[tokio::test]
    async fn chitchat_short_circuits_to_small_tier() {
        let task = AfmDTask {
            state: json!("User message: hi"),
            eligible: vec![
                card_tier("ariacompute/ariamodel-small", "small"),
                card_tier("ariacompute/ariamodel-mid", "mid"),
                card_tier("ariacompute/ariamodel-large", "large"),
            ],
            instructions: DEFAULT_INSTRUCTIONS.into(),
            descriptions: HashMap::new(),
            timeout_ms: 2000,
            min_confidence: None,
            fallback: None,
        };
        let d = AfmDDecisioner {
            // Would fail if we actually called this endpoint.
            endpoint: Some("http://127.0.0.1:1".into()),
        }
        .route(task)
        .await
        .unwrap();
        assert_eq!(d.model, "ariacompute/ariamodel-small");
        assert_eq!(d.reason, "afm-d:chitchat-small");
    }
}
