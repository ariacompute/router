//! In-memory six-factor cost ledger.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_EVENTS: usize = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct CostEvent {
    pub ts: String,
    pub user: String,
    pub key_id: Option<String>,
    pub key_name: Option<String>,
    pub session: String,
    pub entrypoint: String,
    pub layer: String,
    pub decision: String,
    pub model: String,
    pub bypass: bool,
    pub turns_in_request: u32,
    pub upstream_requests: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cost_usd: f64,
    pub tokens_source: String,
    pub priced: bool,
}

#[derive(Debug, Default)]
struct Bucket {
    requests: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    cost_usd: f64,
    priced_requests: u64,
}

#[derive(Debug, Default)]
pub struct CostLedger {
    events: Vec<CostEvent>,
    /// session_id -> successful chat count
    session_turns: HashMap<String, u64>,
    users: HashSet<String>,
    sessions: HashSet<String>,
    totals: Bucket,
    by_model: HashMap<String, Bucket>,
    by_layer: HashMap<String, Bucket>,
    by_entrypoint: HashMap<String, Bucket>,
    by_key: HashMap<String, Bucket>,
}

impl CostLedger {
    pub fn record(&mut self, mut ev: CostEvent) {
        *self.session_turns.entry(ev.session.clone()).or_insert(0) += 1;
        self.users.insert(ev.user.clone());
        self.sessions.insert(ev.session.clone());
        self.bump(&mut ev);
        self.events.push(ev);
        if self.events.len() > MAX_EVENTS {
            self.events.remove(0);
        }
    }

    fn bump(&mut self, ev: &CostEvent) {
        Self::add_bucket(&mut self.totals, ev);
        Self::add_bucket(self.by_model.entry(ev.model.clone()).or_default(), ev);
        Self::add_bucket(self.by_layer.entry(ev.layer.clone()).or_default(), ev);
        Self::add_bucket(
            self.by_entrypoint.entry(ev.entrypoint.clone()).or_default(),
            ev,
        );
        if let Some(kid) = &ev.key_id {
            let label = format!(
                "{}|{}",
                kid,
                ev.key_name.as_deref().unwrap_or("")
            );
            Self::add_bucket(self.by_key.entry(label).or_default(), ev);
        }
    }

    fn add_bucket(b: &mut Bucket, ev: &CostEvent) {
        b.requests += 1;
        b.prompt_tokens += ev.prompt_tokens;
        b.completion_tokens += ev.completion_tokens;
        b.cost_usd += ev.cost_usd;
        if ev.priced {
            b.priced_requests += 1;
        }
    }

    pub fn summary(&self) -> Value {
        let req = self.totals.requests.max(1);
        let tokens = self.totals.prompt_tokens + self.totals.completion_tokens;
        json!({
            "cost_usd": self.totals.cost_usd,
            "requests": self.totals.requests,
            "distinct_users": self.users.len(),
            "avg_tokens_per_request": tokens as f64 / req as f64,
        })
    }

    pub fn report(&self, recent_n: usize) -> Value {
        let n = recent_n.clamp(1, 100);
        let requests = self.totals.requests as f64;
        let users = self.users.len().max(1) as f64;
        let sessions = self.sessions.len().max(1) as f64;
        let turns: u64 = self.session_turns.values().sum();
        let tokens = (self.totals.prompt_tokens + self.totals.completion_tokens) as f64;
        let avg_s_per_u = if self.users.is_empty() {
            0.0
        } else {
            sessions / users
        };
        let avg_t_per_s = if self.sessions.is_empty() {
            0.0
        } else {
            turns as f64 / sessions
        };
        let avg_r_per_t = if turns == 0 {
            0.0
        } else {
            // each recorded event is one turn with upstream_requests
            let up: u64 = self.events.iter().map(|e| e.upstream_requests as u64).sum();
            up as f64 / turns as f64
        };
        let avg_k_per_r = if requests == 0.0 {
            0.0
        } else {
            tokens / requests
        };
        let avg_p = if tokens == 0.0 {
            0.0
        } else {
            self.totals.cost_usd / (tokens / 1_000_000.0)
        };
        let product = users * avg_s_per_u * avg_t_per_s * avg_r_per_t * avg_k_per_r * (avg_p / 1_000_000.0);
        // product uses $/token via avg_p as $/MTok → divide by 1e6 for $/token; residual vs attributed
        let product_usd = users
            * avg_s_per_u
            * avg_t_per_s
            * avg_r_per_t
            * (avg_k_per_r / 1_000_000.0)
            * avg_p;

        json!({
            "totals": {
                "requests": self.totals.requests,
                "distinct_users": self.users.len(),
                "sessions": self.sessions.len(),
                "turns": turns,
                "prompt_tokens": self.totals.prompt_tokens,
                "completion_tokens": self.totals.completion_tokens,
                "tokens": tokens as u64,
                "cost_usd": self.totals.cost_usd,
                "priced_fraction": if requests == 0.0 {
                    0.0
                } else {
                    self.totals.priced_requests as f64 / requests
                },
            },
            "factors": {
                "users": self.users.len(),
                "sessions_per_user": avg_s_per_u,
                "turns_per_session": avg_t_per_s,
                "requests_per_turn": avg_r_per_t,
                "tokens_per_request": avg_k_per_r,
                "price_per_mtok": avg_p,
                "product_usd": product_usd,
                "attributed_cost_usd": self.totals.cost_usd,
                "residual_usd": self.totals.cost_usd - product_usd,
                "_unused_product": product,
            },
            "by_model": map_buckets(&self.by_model),
            "by_layer": map_buckets(&self.by_layer),
            "by_entrypoint": map_buckets(&self.by_entrypoint),
            "by_key": map_buckets(&self.by_key),
            "recent": self.events.iter().rev().take(n).cloned().collect::<Vec<_>>(),
        })
    }
}

fn map_buckets(m: &HashMap<String, Bucket>) -> Value {
    let mut out = serde_json::Map::new();
    for (k, b) in m {
        out.insert(
            k.clone(),
            json!({
                "requests": b.requests,
                "prompt_tokens": b.prompt_tokens,
                "completion_tokens": b.completion_tokens,
                "tokens": b.prompt_tokens + b.completion_tokens,
                "cost_usd": b.cost_usd,
                "priced_requests": b.priced_requests,
            }),
        );
    }
    Value::Object(out)
}

pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count().div_ceil(4)) as u64
}

pub fn cost_usd(prompt: u64, completion: u64, in_mtok: f64, out_mtok: f64) -> f64 {
    (prompt as f64 / 1_000_000.0) * in_mtok + (completion as f64 / 1_000_000.0) * out_mtok
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
