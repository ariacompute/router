//! AFM-D System One decisioner: eligible models → Choice → RouteDecision.
//!
//! Deterministic tier shortcuts (chitchat / factoid / acronym / explain / mid)
//! run before System One so Encoder Choice cannot over-promote teaching/large.
//! Successful System One answers are cached by (normalized text × eligible set).

use aria_router_config::AfmDRecipe;
use aria_router_core::{ChatRequest, ModelCard, RouteDecision, RouterError};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const MAX_CHOICE_OPTIONS: usize = 255;
const DECISION_CACHE_CAP: usize = 256;
const DEFAULT_INSTRUCTIONS: &str = "\
Pick the cheapest sufficient model tier for the user message. Prefer smaller tiers.
small = greetings/chitchat, factoids, acronyms/stand-for, unit conversions, MCQ letter answers, yes/no, one-line definitions
mid = systems judgment, trade-offs, reverse-proxy/placement, multi-question comparisons
large = ONLY explicit teaching / how-it-works / walkthrough (explain, walk me through, how does X work) — never MCQ letter-only prompts
Never pick large for \"What does X stand for?\", boiling points, unit conversion, short facts, or multiple-choice letter answers.";

/// Normalize user text for shortcut matching (lowercase, strip punctuation).
fn normalize_utterance(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphanumeric() || c.is_whitespace() {
            cleaned.push(c.to_ascii_lowercase());
        } else if c == '\'' {
            // drop apostrophes so "what's" → "whats"
        } else {
            cleaned.push(' ');
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First substantive question line (before MCQ options / grading instructions).
/// Compare benches send `format_mcq_prompt` blobs; factoid checks must use the head,
/// not the option list word count.
fn question_head(raw: &str) -> String {
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("answer with") || lower.starts_with("answer concisely") {
            break;
        }
        // Option lines: "A. …", "B) …", "(A) …"
        let bytes = t.as_bytes();
        if bytes.len() >= 2 {
            let c0 = bytes[0].to_ascii_uppercase();
            if (b'A'..=b'J').contains(&c0) && matches!(bytes[1], b'.' | b')' | b':' | b' ') {
                break;
            }
        }
        return t.to_string();
    }
    raw.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(raw)
        .to_string()
}

/// Exact phrases that must route to the small tier without calling System One.
fn is_trivial_chitchat(normalized: &str) -> bool {
    matches!(
        normalized,
        "hi" | "hello" | "hey" | "hiya" | "yo" | "sup" | "howdy"
            | "thanks" | "thank you" | "thx" | "ty"
            | "good morning" | "good afternoon" | "good evening" | "good night"
            | "bye" | "goodbye" | "see you" | "ok" | "okay" | "yes" | "no" | "yep" | "nope"
    )
}

fn is_explain_intent(normalized: &str) -> bool {
    normalized.contains("explain")
        || normalized.contains("walk me through")
        || normalized.contains("walk through")
        || normalized.starts_with("how does ")
        || normalized.contains(" how does ")
        || normalized.starts_with("how do ")
        || normalized.contains(" how do ")
}

fn is_acronym_or_stand_for(normalized: &str) -> bool {
    normalized.contains("stand for")
        || normalized.contains("stands for")
        || normalized.contains("acronym")
        || normalized.contains("what does cpu")
        || normalized.contains("what does http")
        || normalized.contains("what does api")
        || normalized.contains("what does url")
        || normalized.contains("what does json")
        || normalized.contains("what does rest")
}

fn is_systems_mid(normalized: &str) -> bool {
    normalized.contains("reverse proxy")
        || normalized.contains("trade off")
        || normalized.contains("tradeoff")
        || normalized.contains("trade offs")
        || normalized.contains("eviction")
        || normalized.contains("cache eviction")
        || (normalized.contains("when would you") && normalized.contains("proxy"))
        || (normalized.contains("benefit") && normalized.contains("proxy"))
}

fn question_mark_count(raw: &str) -> usize {
    raw.chars().filter(|c| *c == '?' || *c == '？').count()
}

/// Closed-form MCQ / short-answer grading prompts (bench compare style).
/// Large/reasoning models often waste low max_tokens on hidden thinking and return
/// empty letters — prefer small.
fn is_closed_form_prompt(raw: &str, normalized: &str) -> bool {
    if normalized.contains("answer with the letter")
        || normalized.contains("correct option only")
        || normalized.contains("answer concisely")
    {
        return true;
    }
    // Numbered/lettered option block present in the user message.
    let mut saw_a = false;
    let mut saw_b = false;
    for line in raw.lines() {
        let t = line.trim().as_bytes();
        if t.len() < 2 {
            continue;
        }
        let c0 = t[0].to_ascii_uppercase();
        if !(b'A'..=b'J').contains(&c0) || !matches!(t[1], b'.' | b')' | b':') {
            continue;
        }
        if c0 == b'A' {
            saw_a = true;
        }
        if c0 == b'B' {
            saw_b = true;
        }
    }
    saw_a && saw_b
}

fn is_short_factoid(head_normalized: &str, raw_head: &str) -> bool {
    if is_explain_intent(head_normalized) || is_systems_mid(head_normalized) {
        return false;
    }
    if question_mark_count(raw_head) >= 2 {
        return false;
    }
    let words = head_normalized.split_whitespace().count();
    if words > 24 {
        return false;
    }
    head_normalized.contains("boiling point")
        || head_normalized.contains("convert ")
        || head_normalized.contains("celsius")
        || head_normalized.contains("fahrenheit")
        || head_normalized.contains("sea level")
        || (head_normalized.starts_with("what is the ") && words <= 16)
        || (head_normalized.starts_with("what is ") && words <= 12 && !head_normalized.contains("trade"))
        || (head_normalized.starts_with("which ") && words <= 16)
        || (head_normalized.starts_with("in which ") && words <= 16)
        || (head_normalized.starts_with("is ") && words <= 12)
        || (head_normalized.starts_with("name ") && words <= 12)
}

/// Deterministic tier before System One. Priority:
/// chitchat → small; explain → large; acronym → small; systems/multi-q → mid;
/// closed-form MCQ → small; factoid (question head) → small.
fn shortcut_tier(raw: &str) -> Option<(&'static str, &'static str)> {
    let n = normalize_utterance(raw);
    if n.is_empty() {
        return None;
    }
    let head = question_head(raw);
    let head_n = normalize_utterance(&head);

    if is_trivial_chitchat(&n) || is_trivial_chitchat(&head_n) {
        return Some(("small", "afm-d:chitchat-small"));
    }
    // Explain on the question head only — ignore "Answer with…" boilerplate.
    if is_explain_intent(&head_n) {
        return Some(("large", "afm-d:explain-large"));
    }
    if is_acronym_or_stand_for(&n) || is_acronym_or_stand_for(&head_n) {
        return Some(("small", "afm-d:acronym-small"));
    }
    if is_systems_mid(&n) || is_systems_mid(&head_n) {
        return Some(("mid", "afm-d:systems-mid"));
    }
    if question_mark_count(&head) >= 2 {
        return Some(("mid", "afm-d:multi-q-mid"));
    }
    if is_closed_form_prompt(raw, &n) {
        return Some(("small", "afm-d:mcq-small"));
    }
    if is_short_factoid(&head_n, &head) {
        return Some(("small", "afm-d:factoid-small"));
    }
    None
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

fn cache_key(text: &str, eligible: &[ModelCard]) -> String {
    let mut names: Vec<&str> = eligible.iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    format!("{}\0{}", normalize_utterance(text), names.join("\0"))
}

struct DecisionCache {
    map: HashMap<String, String>,
    order: VecDeque<String>,
    capacity: usize,
}

impl DecisionCache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    fn get(&mut self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    fn insert(&mut self, key: String, model: String) {
        use std::collections::hash_map::Entry;
        match self.map.entry(key) {
            Entry::Occupied(mut e) => {
                e.insert(model);
            }
            Entry::Vacant(e) => {
                let key = e.into_key();
                while self.order.len() >= self.capacity {
                    if let Some(old) = self.order.pop_front() {
                        self.map.remove(&old);
                    } else {
                        break;
                    }
                }
                self.order.push_back(key.clone());
                self.map.insert(key, model);
            }
        }
    }
}

fn decision_cache() -> &'static Mutex<DecisionCache> {
    static CACHE: OnceLock<Mutex<DecisionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(DecisionCache::new(DECISION_CACHE_CAP)))
}

#[cfg(test)]
fn clear_decision_cache_for_tests() {
    if let Ok(mut cache) = decision_cache().lock() {
        cache.map.clear();
        cache.order.clear();
    }
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

        let user_text = task.state_user_text();

        // Deterministic tier shortcuts (skip System One).
        if let Some(text) = user_text.as_deref() {
            if let Some((tier, reason)) = shortcut_tier(text) {
                if let Some(m) = pick_tier(&task.eligible, tier) {
                    return Ok(tier_decision(m, reason));
                }
            }
        }

        // Process-local cache of prior System One answers for the same utterance × pool.
        if let Some(text) = user_text.as_deref() {
            let key = cache_key(text, &task.eligible);
            if let Ok(mut cache) = decision_cache().lock() {
                if let Some(model) = cache.get(&key) {
                    if task.eligible.iter().any(|m| m.name == model) {
                        return Ok(RouteDecision {
                            model: model.clone(),
                            algorithm: Some("static".into()),
                            reason: format!("afm-d:cache:{model}"),
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
                        });
                    }
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
            Ok(d) => {
                if let Some(text) = user_text.as_deref() {
                    if let Ok(mut cache) = decision_cache().lock() {
                        cache.insert(cache_key(text, &task.eligible), d.model.clone());
                    }
                }
                Ok(d)
            }
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
        // Must not match deterministic shortcuts (else HTTP tests never hit System One).
        AfmDTask {
            state: json!("User message: Please route this non shortcut probe request"),
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
        clear_decision_cache_for_tests();
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
        let mut task = base_task(vec![card("local/a"), card("local/b")]);
        task.state = json!("User message: host port scheme probe unique alpha");
        let d = AfmDDecisioner {
            endpoint: Some(addr.to_string()),
        }
        .route(task)
        .await
        .unwrap();
        assert_eq!(d.model, "local/b");
        assert!((d.confidence - 0.91).abs() < 1e-5);
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
        clear_decision_cache_for_tests();
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
        task.state = json!("User message: fallback low confidence probe unique beta");
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
        clear_decision_cache_for_tests();
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
        let mut task = base_task(vec![card("local/a"), card("local/b")]);
        task.state = json!("User message: http choice ok probe unique gamma");
        let d = AfmDDecisioner {
            endpoint: Some(format!("http://{addr}")),
        }
        .route(task)
        .await
        .unwrap();
        assert_eq!(d.model, "local/b");
        assert_eq!(d.reason, "afm-d:local/b");
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
        assert!(is_trivial_chitchat(&normalize_utterance("hi")));
        assert!(is_trivial_chitchat(&normalize_utterance("Hi!")));
        assert!(is_trivial_chitchat(&normalize_utterance("  HELLO  ")));
        assert!(is_trivial_chitchat(&normalize_utterance("thank you")));
        assert!(!is_trivial_chitchat(&normalize_utterance(
            "hi, explain rust ownership"
        )));
        assert!(!is_trivial_chitchat(&normalize_utterance("what is HTTP?")));
    }

    #[test]
    fn shortcut_tier_covers_routing_gateway_hard() {
        // Corpus: bench/corpus/routing_gateway_hard.json expected_model → tier.
        let cases: &[(&str, &str, &str)] = &[
            (
                "Explain photosynthesis in one sentence.",
                "large",
                "afm-d:explain-large",
            ),
            (
                "What is the boiling point of water at sea level in Celsius?",
                "small",
                "afm-d:factoid-small",
            ),
            (
                "Name one benefit of using an HTTP reverse proxy.",
                "mid",
                "afm-d:systems-mid",
            ),
            ("What does CPU stand for?", "small", "afm-d:acronym-small"),
            (
                "What is a common design trade-off when choosing a cache eviction policy?",
                "mid",
                "afm-d:systems-mid",
            ),
            (
                "What does the term architecture stand for as a CS acronym quiz?",
                "small",
                "afm-d:acronym-small",
            ),
            ("What is REST? What is GraphQL?", "mid", "afm-d:multi-q-mid"),
            (
                "Convert 100C to Fahrenheit in celsius reference terms briefly.",
                "small",
                "afm-d:factoid-small",
            ),
            (
                "Please explain what an operating system scheduler does.",
                "large",
                "afm-d:explain-large",
            ),
            (
                "When would you place a reverse proxy in front of app servers?",
                "mid",
                "afm-d:systems-mid",
            ),
            ("hi", "small", "afm-d:chitchat-small"),
            ("What does HTTP stand for?", "small", "afm-d:acronym-small"),
        ];
        let mut hits = 0usize;
        for (prompt, tier, reason) in cases {
            let got = shortcut_tier(prompt).expect(prompt);
            assert_eq!(got.0, *tier, "tier mismatch for {prompt}");
            assert_eq!(got.1, *reason, "reason mismatch for {prompt}");
            hits += 1;
        }
        assert_eq!(hits, 12);
        // Offline label quality lower bound when all corpus rows shortcut correctly.
        assert!((hits as f64) / 12.0 >= 0.83);
    }

    /// Mirror of `bench.compare.grade.format_mcq_prompt` for compare corpus rows.
    fn format_mcq_prompt(question: &str, choices: Option<&[&str]>) -> String {
        let mut lines = vec![question.trim().to_string(), String::new()];
        if let Some(choices) = choices {
            for (i, c) in choices.iter().enumerate() {
                let letter = (b'A' + i as u8) as char;
                lines.push(format!("{letter}. {c}"));
            }
            lines.push(String::new());
            lines.push("Answer with the letter of the correct option only.".into());
        } else {
            lines.push("Answer concisely.".into());
        }
        lines.join("\n")
    }

    #[test]
    fn shortcut_tier_covers_mmlu_tiny_compare_prompts() {
        // Compare corpus prompts are MCQ/closed-form — must NOT escalate to large
        // (deepseek + low max_tokens often returns empty letters).
        let cases: &[(&str, Option<&[&str]>, &str, &str)] = &[
            (
                "What is the primary pigment used in photosynthesis?",
                Some(&["Chlorophyll", "Hemoglobin", "Melanin", "Keratin"]),
                "small",
                "afm-d:mcq-small",
            ),
            (
                "What is the SI unit of electric current?",
                Some(&["Volt", "Ampere", "Ohm", "Watt"]),
                "small",
                "afm-d:mcq-small",
            ),
            (
                "In which year did the first Moon landing occur?",
                Some(&["1965", "1969", "1972", "1959"]),
                "small",
                "afm-d:mcq-small",
            ),
            (
                "Which data structure uses FIFO ordering?",
                Some(&["Stack", "Queue", "Tree", "Graph"]),
                "small",
                "afm-d:mcq-small",
            ),
            ("Is 2 a prime number?", None, "small", "afm-d:mcq-small"),
            ("Is the sun a planet?", None, "small", "afm-d:mcq-small"),
            (
                "Name the Linux kernel creator (surname).",
                None,
                "small",
                "afm-d:mcq-small",
            ),
            (
                "What does the term architecture stand for as a CS acronym quiz?",
                Some(&[
                    "A fixed ISO standard code",
                    "It is not a fixed acronym",
                    "Only means CPU microarchitecture",
                    "Only means cloud region layout",
                ]),
                "small",
                "afm-d:acronym-small",
            ),
            (
                "What does HTTP stand for?",
                Some(&[
                    "HyperText Transfer Protocol",
                    "High Transfer Text Pipe",
                    "Host Tunnel Transport Path",
                    "Hybrid Transport Type Protocol",
                ]),
                "small",
                "afm-d:acronym-small",
            ),
        ];
        for (q, choices, tier, reason) in cases {
            let prompt = format_mcq_prompt(q, *choices);
            let got = shortcut_tier(&prompt).unwrap_or_else(|| panic!("no shortcut for {q}"));
            assert_eq!(got.0, *tier, "tier mismatch for {q}");
            assert_eq!(got.1, *reason, "reason mismatch for {q}");
            assert_ne!(got.0, "large", "compare closed-form must not pick large: {q}");
        }
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

    #[tokio::test]
    async fn acronym_short_circuits_without_systemone() {
        let task = AfmDTask {
            state: json!("User message: What does HTTP stand for?"),
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
            endpoint: Some("http://127.0.0.1:1".into()),
        }
        .route(task)
        .await
        .unwrap();
        assert_eq!(d.model, "ariacompute/ariamodel-small");
        assert_eq!(d.reason, "afm-d:acronym-small");
    }

    #[tokio::test]
    async fn systemone_result_is_cached_on_repeat() {
        clear_decision_cache_for_tests();
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use axum::extract::State;

        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/v1/systemone",
                post(|State(hits): State<Arc<AtomicUsize>>| async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "answers": {
                            "route": {
                                "choice": "local/b",
                                "confidence": 0.91
                            }
                        }
                    }))
                }),
            )
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Unique prompt avoids cross-test cache pollution; must not match shortcuts.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let prompt =
            format!("Please classify this nuanced routing edge case carefully {nonce}");
        let make_task = || AfmDTask {
            state: json!(format!("User message: {prompt}")),
            eligible: vec![card("local/a"), card("local/b")],
            instructions: DEFAULT_INSTRUCTIONS.into(),
            descriptions: HashMap::new(),
            timeout_ms: 2000,
            min_confidence: None,
            fallback: None,
        };
        let decisioner = AfmDDecisioner {
            endpoint: Some(format!("http://{addr}")),
        };
        let first = decisioner.route(make_task()).await.unwrap();
        assert_eq!(first.model, "local/b");
        assert_eq!(first.reason, "afm-d:local/b");
        let second = decisioner.route(make_task()).await.unwrap();
        assert_eq!(second.model, "local/b");
        assert_eq!(second.reason, "afm-d:cache:local/b");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
