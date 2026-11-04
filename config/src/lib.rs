//! YAML v0.3 document load + validate.

use aria_router_core::{RouterError, RouterKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterDocument {
    pub version: String,
    #[serde(default)]
    pub listeners: Vec<Listener>,
    #[serde(default)]
    pub providers: Providers,
    #[serde(default)]
    pub entrypoints: Vec<Entrypoint>,
    #[serde(default)]
    pub recipes: Vec<Recipe>,
    #[serde(default)]
    pub global: GlobalCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalCfg {
    #[serde(default = "default_require_api_key")]
    pub require_api_key: bool,
    /// Allow Dashboard self-registration for local users (default true when written by setup).
    #[serde(default = "default_allow_register")]
    pub allow_register: bool,
    #[serde(default)]
    pub keys_path: Option<String>,
    #[serde(default)]
    pub users_path: Option<String>,
    /// Deprecated: OAuth keys live in `keys_path` (`kind: oauth`). Kept for YAML compat.
    #[serde(default)]
    pub serve_account_path: Option<String>,
    /// Emails (case-insensitive) that become router dashboard admins when they
    /// sign in via Aria Compute (serve) OAuth. When empty, the first OAuth user
    /// (only if no admin exists yet) is promoted to admin; everyone else is a
    /// regular user. The serve role is never mirrored to the router role.
    #[serde(default)]
    pub admin_emails: Vec<String>,
    /// Default Aria Compute (serve) site used by the OAuth login flow: "com" or "cn".
    #[serde(default = "default_serve_site")]
    pub serve_site: String,
    /// Optional embedding / classifier catalog for learned signals and semantic_cache.
    #[serde(default)]
    pub model_catalog: HashMap<String, ModelCatalogEntry>,
    #[serde(default)]
    pub stores: StoresCfg,
    #[serde(default)]
    pub services: ServicesCfg,
}

impl Default for GlobalCfg {
    fn default() -> Self {
        Self {
            require_api_key: true,
            allow_register: true,
            keys_path: None,
            users_path: None,
            serve_account_path: None,
            admin_emails: Vec::new(),
            serve_site: default_serve_site(),
            model_catalog: HashMap::new(),
            stores: StoresCfg::default(),
            services: ServicesCfg::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalogEntry {
    #[serde(default = "default_catalog_kind")]
    pub kind: String,
    #[serde(default)]
    pub dim: Option<usize>,
    #[serde(default)]
    pub weight_path: Option<String>,
}

fn default_catalog_kind() -> String {
    "hash".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoresCfg {
    #[serde(default)]
    pub memory: MemoryStoreCfg,
    #[serde(default)]
    pub semantic_cache: SemanticCacheCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryStoreCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    #[serde(default = "default_ttl_turns")]
    pub default_ttl_turns: u32,
}

impl Default for MemoryStoreCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            max_sessions: default_max_sessions(),
            default_ttl_turns: default_ttl_turns(),
        }
    }
}

fn default_max_sessions() -> usize {
    4096
}
fn default_ttl_turns() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCacheCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sim_threshold")]
    pub similarity_threshold: f32,
    #[serde(default = "default_max_cache")]
    pub max_entries: usize,
    #[serde(default = "default_cache_ttl")]
    pub ttl_turns: u32,
}

impl Default for SemanticCacheCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            similarity_threshold: default_sim_threshold(),
            max_entries: default_max_cache(),
            ttl_turns: default_cache_ttl(),
        }
    }
}

fn default_sim_threshold() -> f32 {
    0.92
}
fn default_max_cache() -> usize {
    1024
}
fn default_cache_ttl() -> u32 {
    4
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServicesCfg {
    #[serde(default)]
    pub ratelimit: RateLimitCfg,
    #[serde(default)]
    pub router_replay: RouterReplayCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_rpm")]
    pub requests_per_minute: u32,
}

impl Default for RateLimitCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_minute: default_rpm(),
        }
    }
}

fn default_rpm() -> u32 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterReplayCfg {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_replay_max")]
    pub max_items: usize,
    #[serde(default)]
    pub persist_path: Option<String>,
}

impl Default for RouterReplayCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: default_replay_max(),
            persist_path: None,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_replay_max() -> usize {
    256
}

fn default_require_api_key() -> bool {
    true
}

fn default_allow_register() -> bool {
    true
}

fn default_serve_site() -> String {
    "com".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPricing {
    #[serde(default)]
    pub input_per_mtok: f64,
    #[serde(default)]
    pub output_per_mtok: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listener {
    #[serde(default = "default_listener_name")]
    pub name: String,
    #[serde(default = "default_addr")]
    pub address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub timeout: Option<String>,
}

fn default_listener_name() -> String {
    "http".into()
}
fn default_addr() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8899
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Providers {
    #[serde(default)]
    pub defaults: ProviderDefaults,
    #[serde(default)]
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderDefaults {
    #[serde(default)]
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub name: String,
    #[serde(default)]
    pub provider_model_id: String,
    #[serde(default = "default_locality")]
    pub locality: String,
    #[serde(default = "default_modality")]
    pub modality: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub backend_refs: Vec<BackendRef>,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    /// Explicit pool tier (`small` / `mid` / `large`); preferred over name heuristics.
    #[serde(default)]
    pub tier: Option<String>,
}

impl ProviderModel {
    /// Effective ranking cost for multi-factor (prefer output price, else input).
    pub fn ranking_cost(&self) -> f32 {
        match &self.pricing {
            Some(p) if p.output_per_mtok > 0.0 || p.input_per_mtok > 0.0 => {
                p.output_per_mtok.max(p.input_per_mtok) as f32
            }
            _ => 1.0,
        }
    }
}

fn default_locality() -> String {
    "local".into()
}
fn default_modality() -> String {
    "text".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendRef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
}

fn default_protocol() -> String {
    "http".into()
}
fn default_weight() -> u32 {
    100
}

impl BackendRef {
    pub fn url(&self) -> String {
        if !self.base_url.is_empty() {
            return self.base_url.trim_end_matches('/').to_string();
        }
        let ep = self.endpoint.trim();
        if ep.starts_with("http://") || ep.starts_with("https://") {
            return ep.trim_end_matches('/').to_string();
        }
        format!("http://{ep}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entrypoint {
    pub model_names: Vec<String>,
    pub router: RouterKind,
    pub recipe: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    pub router: RouterKind,
    #[serde(default)]
    pub routing: Option<Routing>,
    #[serde(default)]
    pub agent: Option<AgentRecipe>,
    #[serde(default, rename = "afm-d")]
    pub afm_d: Option<AfmDRecipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routing {
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub model_cards: Option<serde_json::Value>,
    #[serde(default)]
    pub signals: Signals,
    #[serde(default)]
    pub projections: Vec<ProjectionCfg>,
    #[serde(default)]
    pub decisions: Vec<DecisionCfg>,
    #[serde(default)]
    pub algorithms: serde_json::Value,
    #[serde(default)]
    pub plugins: serde_json::Value,
    /// Context-prefix reuse: reuse the frozen decision for the
    /// same (session, turn) instead of re-deciding within an agent turn.
    #[serde(default)]
    pub prefix_reuse: Option<bool>,
    /// Competence-aware downgrade: when enabled and the task is
    /// simple enough with historical competence above `floor`, prefer the
    /// lowest-cost eligible tier instead of the matched model.
    #[serde(default)]
    pub competence: Option<CompetenceCfg>,
    /// Decision-model extension point: delegate the model choice
    /// to an external HTTP decision service.
    #[serde(default)]
    pub decision_ext: Option<DecisionExtCfg>,
}

/// Competence-aware downgrade configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CompetenceCfg {
    #[serde(default)]
    pub enabled: bool,
    /// Historical competence floor (0..1); below this, downgrade is forbidden.
    #[serde(default = "default_competence_floor")]
    pub floor: f32,
    /// Simple-task probability threshold (0..1); at/above this the task is
    /// considered cheap enough to route to a cheaper tier.
    #[serde(default = "default_simple_threshold")]
    pub simple_threshold: f32,
}

fn default_competence_floor() -> f32 {
    0.6
}

fn default_simple_threshold() -> f32 {
    0.7
}

/// Decision-model extension point. Per-recipe (not top-level) so it
/// does not conflict with the repo's deliberate rejection of a top-level `extensions`
/// block. Contract mirrors `afm-d` System One: `endpoint` + `track` + `timeout_ms` + `fallback`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DecisionExtCfg {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub auth: Option<String>,
}

fn default_strategy() -> String {
    "priority".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Signals {
    #[serde(default)]
    pub keywords: Vec<KeywordSignal>,
    #[serde(default)]
    pub language: Vec<NamedSignal>,
    #[serde(default)]
    pub context: Vec<ContextSignal>,
    #[serde(default)]
    pub authz: Vec<NamedSignal>,
    #[serde(default)]
    pub conversation: Vec<ConversationSignal>,
    #[serde(default)]
    pub metadata: Vec<MetadataSignal>,
    #[serde(default)]
    pub event: Vec<NamedSignal>,
    #[serde(default)]
    pub structure: Vec<NamedSignal>,
    /// Learned / extra families (stage C): presence in YAML is allowed; evaluation may be Unsupported.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordSignal {
    pub name: String,
    #[serde(default = "default_or")]
    pub operator: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

fn default_or() -> String {
    "OR".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedSignal {
    pub name: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSignal {
    pub name: String,
    #[serde(default)]
    pub min_tokens: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSignal {
    pub name: String,
    #[serde(default)]
    pub min_messages: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataSignal {
    pub name: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionCfg {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCfg {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub rules: RuleNode,
    #[serde(default, rename = "modelRefs")]
    pub model_refs: Vec<ModelRef>,
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub plugins: Vec<PluginRef>,
    #[serde(default)]
    pub locality: Option<String>,
    #[serde(default)]
    pub emits: Vec<EmitDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitDirective {
    pub kind: String,
    #[serde(default)]
    pub retention: Option<RetentionDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RetentionDirective {
    #[serde(default)]
    pub drop: Option<bool>,
    #[serde(default)]
    pub ttl_turns: Option<u32>,
    #[serde(default)]
    pub keep_current_model: Option<bool>,
    #[serde(default)]
    pub prefer_prefix_retention: Option<bool>,
}

impl RetentionDirective {
    pub fn validate(&self, decision_name: &str) -> Result<(), RouterError> {
        let drop = self.drop.unwrap_or(false);
        let ttl = self.ttl_turns.unwrap_or(0);
        if drop && ttl > 0 {
            return Err(RouterError::Config(format!(
                "decision {decision_name}: retention drop:true conflicts with positive ttl_turns"
            )));
        }
        Ok(())
    }
}

impl DecisionCfg {
    pub fn retention(&self) -> Option<&RetentionDirective> {
        self.emits.iter().find_map(|e| {
            if e.kind == "retention" {
                e.retention.as_ref()
            } else {
                None
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRef {
    pub name: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleNode {
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub not: Option<Box<RuleNode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    /// For `type: projection`, optional expected string value (e.g. band label).
    #[serde(default)]
    pub equals: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRecipe {
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Context-prefix reuse: reuse the frozen decision for the
    /// same (session, turn) instead of re-deciding within an agent turn.
    #[serde(default)]
    pub prefix_reuse: Option<bool>,
    /// Decision-model extension point: delegate the model choice
    /// to an external HTTP decision service.
    #[serde(default)]
    pub decision_ext: Option<DecisionExtCfg>,
    /// Long-horizon agent delegation: route the decision to an
    /// external agent / Worker-plan endpoint instead of the in-process builtin.
    #[serde(default)]
    pub delegate_endpoint: Option<String>,
    /// Optional Worker-plan / track identifier passed to the delegate endpoint.
    #[serde(default)]
    pub worker_plan: Option<String>,
    /// Optional bearer token for the delegate endpoint.
    #[serde(default)]
    pub delegate_auth: Option<String>,
}

/// System One backend track for AFM-D (`afm-d.track`).
///
/// - [`AfmDTrack::Encoder`]: aria-engine `--track encoder` (default port **8010**); Choice pool up to 255.
/// - [`AfmDTrack::Decoder`]: aria-engine `--track decoder` / `afm_d.dd.serve` (default port **8011**); Choice pool **2–16**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AfmDTrack {
    Encoder,
    Decoder,
}

impl AfmDTrack {
    /// Max eligible models allowed when calling System One for this track.
    pub fn max_choice_options(self) -> usize {
        match self {
            Self::Encoder => 255,
            Self::Decoder => 16,
        }
    }
}

impl std::fmt::Display for AfmDTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encoder => write!(f, "encoder"),
            Self::Decoder => write!(f, "decoder"),
        }
    }
}

/// AFM-D System One decisioner recipe (`router: afm-d`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AfmDRecipe {
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Optional `encoder` | `decoder`. When set, eligible-pool size is checked
    /// against the track contract (decoder: ≤16; encoder: ≤255).
    /// Unset → encoder pool ceiling (255). Gateway starter defaults to `encoder` @ :8010.
    #[serde(default)]
    pub track: Option<AfmDTrack>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    /// Optional per-model Choice descriptions (overrides synthesized text).
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    /// If set and answer confidence is below this, use fallback / FailClosed.
    #[serde(default)]
    pub min_confidence: Option<f32>,
    /// Context-prefix reuse: reuse the frozen decision for the
    /// same (session, turn) instead of re-deciding within an agent turn.
    #[serde(default)]
    pub prefix_reuse: Option<bool>,
}

const LEARNED_SIGNAL_KEYS: &[&str] = &[
    "classifier",
    "classifiers",
    "complexity",
    "domain",
    "domains",
    "embedding",
    "embeddings",
    "fact-check",
    "fact_check",
    "fact_checks",
    "jailbreak",
    "jailbreaks",
    "kb",
    "modality",
    "pii",
    "preference",
    "preferences",
    "reask",
    "reasks",
    "user-feedback",
    "user_feedback",
    "user_feedbacks",
];

const IMPLEMENTED_ALGOS: &[&str] = &[
    "static",
    "latency-aware",
    "latency_aware",
    "multi-factor",
    "multi_factor",
    "elo",
    "ratings",
];
const KNOWN_ALGOS: &[&str] = &[
    "static",
    "latency-aware",
    "latency_aware",
    "multi-factor",
    "multi_factor",
    "elo",
    "automix",
    "hybrid",
    "kmeans",
    "knn",
    "mlp",
    "prompt",
    "router-dc",
    "router_dc",
    "svm",
    "confidence",
    "fusion",
    "ratings",
    "remom",
    "workflows",
];

impl RouterDocument {
    pub fn from_yaml_str(raw: &str) -> Result<Self, RouterError> {
        let expanded = expand_env(raw);
        let doc: Self = serde_yaml::from_str(&expanded)
            .map_err(|e| RouterError::Config(format!("yaml: {e}")))?;
        doc.validate()?;
        Ok(doc)
    }

    pub fn from_json_str(raw: &str) -> Result<Self, RouterError> {
        let doc: Self =
            serde_json::from_str(raw).map_err(|e| RouterError::Config(format!("json: {e}")))?;
        doc.validate()?;
        Ok(doc)
    }

    pub fn to_yaml(&self) -> Result<String, RouterError> {
        serde_yaml::to_string(self).map_err(|e| RouterError::Config(format!("yaml: {e}")))
    }

    pub fn load_path(path: impl AsRef<Path>) -> Result<Self, RouterError> {
        let raw = std::fs::read_to_string(path.as_ref())
            .map_err(|e| RouterError::Io(format!("read {}: {e}", path.as_ref().display())))?;
        Self::from_yaml_str(&raw)
    }

    pub fn validate(&self) -> Result<(), RouterError> {
        if self.version != "v0.3" {
            return Err(RouterError::Config(format!(
                "version must be v0.3, got {}",
                self.version
            )));
        }
        let recipe_by_name: HashMap<&str, &Recipe> =
            self.recipes.iter().map(|r| (r.name.as_str(), r)).collect();
        for ep in &self.entrypoints {
            let recipe = recipe_by_name.get(ep.recipe.as_str()).ok_or_else(|| {
                RouterError::Config(format!("entrypoint recipe {} not found", ep.recipe))
            })?;
            if recipe.router != ep.router {
                return Err(RouterError::Config(format!(
                    "entrypoint.router ({}) != recipe.router ({}) for {}",
                    ep.router.as_str(),
                    recipe.router.as_str(),
                    ep.recipe
                )));
            }
        }
        for recipe in &self.recipes {
            match recipe.router {
                RouterKind::Semantic => {
                    if recipe.agent.is_some() {
                        return Err(RouterError::Config(format!(
                            "semantic recipe {} must not contain agent:",
                            recipe.name
                        )));
                    }
                    if recipe.afm_d.is_some() {
                        return Err(RouterError::Config(format!(
                            "semantic recipe {} must not contain afm-d:",
                            recipe.name
                        )));
                    }
                    let routing = recipe.routing.as_ref().ok_or_else(|| {
                        RouterError::Config(format!(
                            "semantic recipe {} missing routing",
                            recipe.name
                        ))
                    })?;
                    self.validate_routing(routing)?;
                }
                RouterKind::Agent => {
                    if recipe.afm_d.is_some() {
                        return Err(RouterError::Config(format!(
                            "agent recipe {} must not contain afm-d:",
                            recipe.name
                        )));
                    }
                    if recipe
                        .routing
                        .as_ref()
                        .is_some_and(|r| !r.signals.keywords.is_empty() || !r.decisions.is_empty())
                    {
                        return Err(RouterError::Config(format!(
                            "agent recipe {} must not contain signals/decisions",
                            recipe.name
                        )));
                    }
                    let agent = recipe.agent.as_ref().ok_or_else(|| {
                        RouterError::Config(format!("agent recipe {} missing agent:", recipe.name))
                    })?;
                    if let Some(fb) = &agent.fallback {
                        if self.provider(fb).is_none() {
                            return Err(RouterError::Config(format!(
                                "agent recipe {} fallback {fb} not in providers",
                                recipe.name
                            )));
                        }
                    }
                    if let Some(t) = agent.max_turns {
                        if t == 0 || t > 8 {
                            return Err(RouterError::Config(format!(
                                "agent recipe {} max_turns must be 1..=8, got {t}",
                                recipe.name
                            )));
                        }
                    }
                }
                RouterKind::AfmD => {
                    if recipe.agent.is_some() {
                        return Err(RouterError::Config(format!(
                            "afm-d recipe {} must not contain agent:",
                            recipe.name
                        )));
                    }
                    if recipe
                        .routing
                        .as_ref()
                        .is_some_and(|r| !r.signals.keywords.is_empty() || !r.decisions.is_empty())
                    {
                        return Err(RouterError::Config(format!(
                            "afm-d recipe {} must not contain signals/decisions",
                            recipe.name
                        )));
                    }
                    let afm = recipe.afm_d.as_ref().ok_or_else(|| {
                        RouterError::Config(format!("afm-d recipe {} missing afm-d:", recipe.name))
                    })?;
                    if let Some(fb) = &afm.fallback {
                        if self.provider(fb).is_none() {
                            return Err(RouterError::Config(format!(
                                "afm-d recipe {} fallback {fb} not in providers",
                                recipe.name
                            )));
                        }
                    }
                    if let Some(c) = afm.min_confidence {
                        if !(0.0..=1.0).contains(&c) {
                            return Err(RouterError::Config(format!(
                                "afm-d recipe {} min_confidence must be 0..=1, got {c}",
                                recipe.name
                            )));
                        }
                    }
                    if let Some(track) = afm.track {
                        // Eligible pool starts from text-capable providers; fail closed
                        // early when the catalog already exceeds the track Choice limit.
                        let text_n = self
                            .providers
                            .models
                            .iter()
                            .filter(|m| {
                                let mod_ = m.modality.trim();
                                mod_.is_empty() || mod_.eq_ignore_ascii_case("text")
                            })
                            .count();
                        let max = track.max_choice_options();
                        if text_n > max {
                            return Err(RouterError::Config(format!(
                                "afm-d recipe {} track={track} allows at most {max} text providers, got {text_n}",
                                recipe.name
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_routing(&self, routing: &Routing) -> Result<(), RouterError> {
        for (key, val) in &routing.signals.extra {
            let learned = LEARNED_SIGNAL_KEYS.contains(&key.as_str());
            if !learned && !key.is_empty() {
                return Err(RouterError::Config(format!("unknown signal family {key}")));
            }
            if !learned {
                continue;
            }
            let referenced = routing.decisions.iter().any(|d| {
                d.rules
                    .conditions
                    .iter()
                    .any(|c| c.kind == *key || c.kind.replace('_', "-") == *key)
            });
            if referenced && !val.is_null() {
                // Stage C: learned types are known but require ml feature / weights.
                // Validation of "referenced without implementation" happens at runtime
                // unless empty. Empty extra is ok; non-empty extra still allowed to parse.
                let _ = referenced;
            }
        }
        for d in &routing.decisions {
            if let Some(algo) = &d.algorithm {
                if !KNOWN_ALGOS.contains(&algo.as_str()) {
                    return Err(RouterError::Unsupported(format!(
                        "unknown algorithm {algo}"
                    )));
                }
            }
            let mut retention_count = 0usize;
            for emit in &d.emits {
                match emit.kind.as_str() {
                    "retention" => {
                        retention_count += 1;
                        let Some(ret) = &emit.retention else {
                            return Err(RouterError::Config(format!(
                                "decision {}: emit retention missing retention block",
                                d.name
                            )));
                        };
                        ret.validate(&d.name)?;
                    }
                    other => {
                        return Err(RouterError::Unsupported(format!(
                            "decision {}: unknown emit kind {other}",
                            d.name
                        )));
                    }
                }
            }
            if retention_count > 1 {
                return Err(RouterError::Config(format!(
                    "decision {}: duplicate emit retention",
                    d.name
                )));
            }
        }
        if let Some(ext) = &routing.decision_ext {
            match ext.endpoint.as_deref() {
                Some(e) if !e.trim().is_empty() => {}
                _ => {
                    return Err(RouterError::Unsupported(
                        "decision_ext requires a non-empty `endpoint`".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn recipe(&self, name: &str) -> Result<&Recipe, RouterError> {
        self.recipes
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| RouterError::Config(format!("recipe {name} not found")))
    }

    pub fn entrypoint_for(&self, model: &str) -> Option<&Entrypoint> {
        self.entrypoints
            .iter()
            .find(|e| e.model_names.iter().any(|n| n == model))
    }

    pub fn provider(&self, name: &str) -> Option<&ProviderModel> {
        self.providers.models.iter().find(|m| m.name == name)
    }

    pub fn is_concrete_model(&self, name: &str) -> bool {
        self.provider(name).is_some()
    }

    pub fn data_bind(&self) -> String {
        let l = self.listeners.first();
        match l {
            Some(l) => format!("{}:{}", l.address, l.port),
            None => "0.0.0.0:8899".into(),
        }
    }

    pub fn learned_signal_referenced(&self, recipe: &Recipe) -> Vec<String> {
        let Some(routing) = &recipe.routing else {
            return vec![];
        };
        let mut out = vec![];
        for d in &routing.decisions {
            for c in &d.rules.conditions {
                if LEARNED_SIGNAL_KEYS.contains(&c.kind.as_str()) {
                    out.push(c.kind.clone());
                }
            }
        }
        out
    }

    pub fn unimplemented_algorithm(name: &str) -> bool {
        KNOWN_ALGOS.contains(&name) && !IMPLEMENTED_ALGOS.contains(&name)
    }
}

pub fn expand_env(raw: &str) -> String {
    // `${VAR}` / `${VAR:-default}` — `:-` matches bash (unset **or empty** → default).
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}").expect("regex");
    re.replace_all(raw, |caps: &regex::Captures| {
        let key = &caps[1];
        let default = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        match std::env::var(key) {
            Ok(v) if !v.is_empty() => v,
            _ => default.to_string(),
        }
    })
    .into_owned()
}

/// Embedded starter templates written by `aria-router setup` (gateway gold path).
pub const SEMANTIC_GATEWAY_YAML: &str = include_str!("../examples/semantic-gateway.yaml");
pub const AGENT_GATEWAY_YAML: &str = include_str!("../examples/agent-gateway.yaml");
pub const AFM_D_GATEWAY_YAML: &str = include_str!("../examples/afm-d-gateway.yaml");

/// `$HOME/.ariacompute` (overridable via `ARIA_COMPUTE_HOME`).
pub fn aria_home() -> Result<PathBuf, RouterError> {
    if let Ok(override_home) = std::env::var("ARIA_COMPUTE_HOME") {
        if !override_home.is_empty() {
            return Ok(PathBuf::from(override_home));
        }
    }
    let home = dirs::home_dir()
        .ok_or_else(|| RouterError::Io("could not resolve home directory".into()))?;
    Ok(home.join(".ariacompute"))
}

pub fn default_config_path() -> Result<PathBuf, RouterError> {
    Ok(aria_home()?.join("router.yml"))
}

pub fn default_keys_path() -> Result<PathBuf, RouterError> {
    Ok(aria_home()?.join("router-keys.json"))
}

pub fn default_users_path() -> Result<PathBuf, RouterError> {
    Ok(aria_home()?.join("router-users.json"))
}

/// FFI install dir shared with SDKs (`ARIA_ROUTER_FFI_LIB` cache).
pub fn lib_dir() -> Result<PathBuf, RouterError> {
    Ok(aria_home()?.join("lib"))
}

/// CLI metadata (not the gateway recipe): `~/.ariacompute/router-cli.yml`.
pub fn default_cli_config_path() -> Result<PathBuf, RouterError> {
    Ok(aria_home()?.join("router-cli.yml"))
}

/// Ensure `~/.ariacompute`, `lib/`, and `tmp/` exist.
pub fn ensure_aria_home() -> Result<PathBuf, RouterError> {
    let home = aria_home()?;
    std::fs::create_dir_all(&home).map_err(|e| RouterError::Io(e.to_string()))?;
    std::fs::create_dir_all(home.join("lib")).map_err(|e| RouterError::Io(e.to_string()))?;
    std::fs::create_dir_all(home.join("tmp")).map_err(|e| RouterError::Io(e.to_string()))?;
    Ok(home)
}

/// Default Releases org root for `.com` (GitHub) / `.cn` (Gitee).
pub const DEFAULT_UPGRADE_URL_COM: &str = "https://github.com/ariacompute";
pub const DEFAULT_UPGRADE_URL_CN: &str = "https://gitee.com/ariacompute";

/// Pick default `upgrade_url` from `serve_site` (`com` | `cn`).
pub fn default_upgrade_url_for_site(serve_site: &str) -> &'static str {
    if serve_site.trim().eq_ignore_ascii_case("cn") {
        DEFAULT_UPGRADE_URL_CN
    } else {
        DEFAULT_UPGRADE_URL_COM
    }
}

/// Sidecar CLI config (separate from recipe `router.yml`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouterCliConfig {
    #[serde(default)]
    pub upgrade_url: String,
}

pub fn load_cli_config() -> Result<RouterCliConfig, RouterError> {
    let path = default_cli_config_path()?;
    if !path.exists() {
        return Ok(RouterCliConfig::default());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| RouterError::Io(e.to_string()))?;
    serde_yaml::from_str(&raw).map_err(|e| RouterError::Config(e.to_string()))
}

pub fn save_cli_config(cfg: &RouterCliConfig) -> Result<PathBuf, RouterError> {
    ensure_aria_home()?;
    let path = default_cli_config_path()?;
    let raw = serde_yaml::to_string(cfg).map_err(|e| RouterError::Config(e.to_string()))?;
    std::fs::write(&path, raw).map_err(|e| RouterError::Io(e.to_string()))?;
    Ok(path)
}

pub fn clear_cli_config() -> Result<Option<PathBuf>, RouterError> {
    let path = default_cli_config_path()?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| RouterError::Io(e.to_string()))?;
        return Ok(Some(path));
    }
    Ok(None)
}

/// Expand `~/` or leave absolute/relative paths as-is under aria home resolution.
pub fn resolve_home_path(
    raw: &str,
    default: fn() -> Result<PathBuf, RouterError>,
) -> Result<PathBuf, RouterError> {
    let t = raw.trim();
    if t.is_empty() {
        return default();
    }
    if let Some(rest) = t.strip_prefix("~/") {
        let home = dirs::home_dir()
            .ok_or_else(|| RouterError::Io("could not resolve home directory".into()))?;
        return Ok(home.join(rest));
    }
    if t.starts_with('/') {
        return Ok(PathBuf::from(t));
    }
    Ok(aria_home()?.join(t))
}

/// Expand `~/` or leave absolute/relative paths as-is under aria home resolution.
pub fn resolve_keys_path(raw: &str) -> Result<PathBuf, RouterError> {
    resolve_home_path(raw, default_keys_path)
}

pub fn resolve_users_path(raw: &str) -> Result<PathBuf, RouterError> {
    resolve_home_path(raw, default_users_path)
}

/// Optional overrides applied on top of the embedded gateway template.
#[derive(Debug, Clone, Default)]
pub struct SetupModelOpts {
    pub base_url: Option<String>,
    /// Env var **name** for upstream auth (default in template: `GATEWAY_API_KEY`).
    pub api_key_env: Option<String>,
    /// Optional secret written into each `backend_refs[].api_key` so serve can auth
    /// from `router.yml` without exporting an env var.
    pub api_key: Option<String>,
    pub small_provider_model_id: Option<String>,
    pub mid_provider_model_id: Option<String>,
    pub large_provider_model_id: Option<String>,
    pub agent_endpoint: Option<String>,
    pub agent_model: Option<String>,
    pub agent_fallback: Option<String>,
    /// AFM-D System One endpoint (`afm-d` template).
    pub afm_d_endpoint: Option<String>,
    pub afm_d_fallback: Option<String>,
}

const LOGICAL_SMALL: &str = "ariacompute/ariamodel-small";
const LOGICAL_MID: &str = "ariacompute/ariamodel-mid";
const LOGICAL_LARGE: &str = "ariacompute/ariamodel-large";

/// Embedded gateway YAML for a setup template (`semantic` / `agent` / `afm-d`).
pub fn gateway_template_yaml(kind: &str) -> Result<&'static str, RouterError> {
    match kind {
        "" | "semantic" => Ok(SEMANTIC_GATEWAY_YAML),
        "agent" => Ok(AGENT_GATEWAY_YAML),
        "afm-d" => Ok(AFM_D_GATEWAY_YAML),
        other => Err(RouterError::InvalidParam(format!(
            "setup template must be semantic|agent|afm-d, got {other}"
        ))),
    }
}

/// Upstream `provider_model_id` defaults from the embedded gateway template
/// (small, mid, large). Callers should use these as setup prompt hints instead
/// of hardcoding TokenHub model names in the CLI.
pub fn template_provider_model_ids(kind: &str) -> Result<(String, String, String), RouterError> {
    let doc = RouterDocument::from_yaml_str(gateway_template_yaml(kind)?)?;
    provider_model_ids_from(&doc, kind)
}

fn provider_model_ids_from(
    doc: &RouterDocument,
    kind: &str,
) -> Result<(String, String, String), RouterError> {
    let id = |logical: &str| -> Result<String, RouterError> {
        doc.provider(logical)
            .map(|m| m.provider_model_id.clone())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                RouterError::Config(format!(
                    "template {kind:?} missing provider_model_id for {logical}"
                ))
            })
    };
    Ok((id(LOGICAL_SMALL)?, id(LOGICAL_MID)?, id(LOGICAL_LARGE)?))
}

fn yaml_str(s: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(s.to_string())
}

fn patch_setup_models(doc: &mut serde_yaml::Value, opts: &SetupModelOpts) {
    let Some(root) = doc.as_mapping_mut() else {
        return;
    };
    if let Some(providers) = root
        .get_mut(serde_yaml::Value::String("providers".into()))
        .and_then(|v| v.as_mapping_mut())
    {
        if let Some(models) = providers
            .get_mut(serde_yaml::Value::String("models".into()))
            .and_then(|v| v.as_sequence_mut())
        {
            for model in models.iter_mut() {
                let Some(m) = model.as_mapping_mut() else {
                    continue;
                };
                let name = m
                    .get(serde_yaml::Value::String("name".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let upstream = match name {
                    LOGICAL_SMALL => opts.small_provider_model_id.as_deref(),
                    LOGICAL_MID => opts.mid_provider_model_id.as_deref(),
                    LOGICAL_LARGE => opts.large_provider_model_id.as_deref(),
                    _ => None,
                };
                if let Some(id) = upstream {
                    m.insert(
                        serde_yaml::Value::String("provider_model_id".into()),
                        yaml_str(id),
                    );
                }
                if let Some(refs) = m
                    .get_mut(serde_yaml::Value::String("backend_refs".into()))
                    .and_then(|v| v.as_sequence_mut())
                {
                    for r in refs.iter_mut() {
                        let Some(rm) = r.as_mapping_mut() else {
                            continue;
                        };
                        if let Some(url) = opts.base_url.as_deref() {
                            rm.insert(serde_yaml::Value::String("base_url".into()), yaml_str(url));
                        }
                        if let Some(env) = opts.api_key_env.as_deref() {
                            rm.insert(
                                serde_yaml::Value::String("api_key_env".into()),
                                yaml_str(env),
                            );
                        }
                        if let Some(key) = opts.api_key.as_deref() {
                            rm.insert(serde_yaml::Value::String("api_key".into()), yaml_str(key));
                        }
                    }
                }
            }
        }
    }

    let agent_endpoint = opts
        .agent_endpoint
        .clone()
        .or_else(|| opts.base_url.clone());
    let afm_d_endpoint = opts.afm_d_endpoint.clone();
    let patch_agent =
        agent_endpoint.is_some() || opts.agent_model.is_some() || opts.agent_fallback.is_some();
    let patch_afm_d = afm_d_endpoint.is_some() || opts.afm_d_fallback.is_some();
    if !patch_agent && !patch_afm_d {
        return;
    }
    let Some(recipes) = root
        .get_mut(serde_yaml::Value::String("recipes".into()))
        .and_then(|v| v.as_sequence_mut())
    else {
        return;
    };
    for recipe in recipes.iter_mut() {
        let Some(rm) = recipe.as_mapping_mut() else {
            continue;
        };
        let router = rm
            .get(serde_yaml::Value::String("router".into()))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if patch_agent && router == "agent" {
            if let Some(agent) = rm
                .get_mut(serde_yaml::Value::String("agent".into()))
                .and_then(|v| v.as_mapping_mut())
            {
                if let Some(ep) = agent_endpoint.as_deref() {
                    agent.insert(serde_yaml::Value::String("endpoint".into()), yaml_str(ep));
                }
                if let Some(model) = opts.agent_model.as_deref() {
                    agent.insert(serde_yaml::Value::String("model".into()), yaml_str(model));
                }
                if let Some(fb) = opts.agent_fallback.as_deref() {
                    agent.insert(serde_yaml::Value::String("fallback".into()), yaml_str(fb));
                }
            }
        }
        if patch_afm_d && router == "afm-d" {
            if let Some(afm) = rm
                .get_mut(serde_yaml::Value::String("afm-d".into()))
                .and_then(|v| v.as_mapping_mut())
            {
                if let Some(ep) = afm_d_endpoint.as_deref() {
                    afm.insert(serde_yaml::Value::String("endpoint".into()), yaml_str(ep));
                }
                if let Some(fb) = opts.afm_d_fallback.as_deref() {
                    afm.insert(serde_yaml::Value::String("fallback".into()), yaml_str(fb));
                }
            }
        }
    }
}

/// Write a v0.3 starter YAML to `~/.ariacompute/router.yml`.
/// `kind` is `semantic` (default), `agent`, or `afm-d`.
pub fn write_default_config(kind: &str, overwrite: bool) -> Result<PathBuf, RouterError> {
    write_default_config_with(kind, overwrite, true, true)
}

pub struct SetupGlobalOpts {
    pub require_api_key: bool,
    pub allow_register: bool,
}

/// Like [`write_default_config`], with local auth globals.
pub fn write_default_config_with(
    kind: &str,
    overwrite: bool,
    require_api_key: bool,
    allow_register: bool,
) -> Result<PathBuf, RouterError> {
    write_default_config_with_opts(
        kind,
        overwrite,
        require_api_key,
        allow_register,
        &SetupModelOpts::default(),
    )
}

/// Write gateway starter YAML with optional model/backend overrides.
pub fn write_default_config_with_opts(
    kind: &str,
    overwrite: bool,
    require_api_key: bool,
    allow_register: bool,
    models: &SetupModelOpts,
) -> Result<PathBuf, RouterError> {
    let path = default_config_path()?;
    if path.exists() && !overwrite {
        return Err(RouterError::Io(format!(
            "{} exists (pass overwrite or --clear)",
            path.display()
        )));
    }
    let body = gateway_template_yaml(kind)?;
    let keys = default_keys_path()?;
    let users = default_users_path()?;
    let disp = |p: &PathBuf| {
        format!(
            "~/.ariacompute/{}",
            p.file_name().and_then(|s| s.to_str()).unwrap_or("file")
        )
    };
    let mut doc: serde_yaml::Value = serde_yaml::from_str(body)
        .map_err(|e| RouterError::Config(format!("embedded template: {e}")))?;
    patch_setup_models(&mut doc, models);
    if let Some(map) = doc.as_mapping_mut() {
        let mut g = serde_yaml::Mapping::new();
        g.insert(
            serde_yaml::Value::String("require_api_key".into()),
            serde_yaml::Value::Bool(require_api_key),
        );
        g.insert(
            serde_yaml::Value::String("allow_register".into()),
            serde_yaml::Value::Bool(allow_register),
        );
        g.insert(
            serde_yaml::Value::String("keys_path".into()),
            serde_yaml::Value::String(disp(&keys)),
        );
        g.insert(
            serde_yaml::Value::String("users_path".into()),
            serde_yaml::Value::String(disp(&users)),
        );
        g.insert(
            serde_yaml::Value::String("serve_site".into()),
            serde_yaml::Value::String(default_serve_site()),
        );
        g.insert(
            serde_yaml::Value::String("admin_emails".into()),
            serde_yaml::Value::Sequence(vec![]),
        );
        map.insert(
            serde_yaml::Value::String("global".into()),
            serde_yaml::Value::Mapping(g),
        );
    }
    let out = serde_yaml::to_string(&doc).map_err(|e| RouterError::Config(e.to_string()))?;
    std::fs::create_dir_all(aria_home()?)?;
    std::fs::write(&path, out)?;
    if !keys.exists() {
        std::fs::write(&keys, "{\n  \"keys\": []\n}\n")?;
    }
    if !users.exists() {
        std::fs::write(&users, "{\n  \"users\": []\n}\n")?;
    }
    RouterDocument::load_path(&path)?;
    Ok(path)
}

pub fn clear_default_config() -> Result<PathBuf, RouterError> {
    let path = default_config_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Serialize env mutation and always restore `ARIA_COMPUTE_HOME`.
    fn with_aria_compute_home<T>(f: impl FnOnce(&Path) -> T) -> T {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
        with_aria_compute_home_locked(prev, f)
    }

    /// Body of [`with_aria_compute_home`] with an explicit previous value, so the
    /// "restore a pre-existing override" branch is testable without env races.
    fn with_aria_compute_home_locked<T>(prev: Option<String>, f: impl FnOnce(&Path) -> T) -> T {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ARIA_COMPUTE_HOME", dir.path());
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(dir.path())));
        restore_env(prev);
        match out {
            Ok(v) => v,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Restore `ARIA_COMPUTE_HOME` to a previously captured value.
    fn restore_env(prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var("ARIA_COMPUTE_HOME", v),
            None => std::env::remove_var("ARIA_COMPUTE_HOME"),
        }
    }

    const TINY: &str = r#"
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
"#;

    #[test]
    fn parse_competence_cfg_explicit_values() {
        let doc = RouterDocument::from_yaml_str(
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
      competence:
        enabled: true
        floor: 0.5
        simple_threshold: 0.8
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
"#,
        )
        .unwrap();
        let comp = doc.recipes[0]
            .routing
            .as_ref()
            .unwrap()
            .competence
            .as_ref()
            .unwrap();
        assert!(comp.enabled);
        assert!((comp.floor - 0.5).abs() < 1e-6);
        assert!((comp.simple_threshold - 0.8).abs() < 1e-6);
    }

    #[test]
    fn parse_competence_cfg_defaults() {
        let doc = RouterDocument::from_yaml_str(
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
      competence:
        enabled: true
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
"#,
        )
        .unwrap();
        let comp = doc.recipes[0]
            .routing
            .as_ref()
            .unwrap()
            .competence
            .as_ref()
            .unwrap();
        // Defaults: floor 0.6, simple_threshold 0.7.
        assert!((comp.floor - 0.6).abs() < 1e-6);
        assert!((comp.simple_threshold - 0.7).abs() < 1e-6);
    }

    #[test]
    fn parse_tiny() {
        let doc = RouterDocument::from_yaml_str(TINY).unwrap();
        assert_eq!(doc.entrypoints[0].router, RouterKind::Semantic);
    }

    #[test]
    fn env_expand() {
        std::env::set_var("ARIA_TEST_KEY", "secret");
        let s = expand_env("k: ${ARIA_TEST_KEY}");
        assert!(s.contains("secret"));
    }

    #[test]
    fn env_expand_colon_dash_uses_default_when_empty() {
        std::env::set_var("ARIA_TEST_EMPTY", "");
        let s = expand_env("ep: ${ARIA_TEST_EMPTY:-http://127.0.0.1:8011}");
        assert!(
            s.contains("http://127.0.0.1:8011"),
            "empty env with :- must fall back to default, got {s}"
        );
        std::env::remove_var("ARIA_TEST_EMPTY");
        let s2 = expand_env("ep: ${ARIA_TEST_UNSET_XYZ:-http://127.0.0.1:8011}");
        assert!(s2.contains("http://127.0.0.1:8011"));
    }

    #[test]
    fn mismatch_router_fails() {
        let raw = TINY.replace("router: semantic\n    recipe", "router: agent\n    recipe");
        assert!(RouterDocument::from_yaml_str(&raw).is_err());
    }

    #[test]
    fn unknown_top_level_fails() {
        let raw = format!("{TINY}\nunknown_block: true\n");
        let err = RouterDocument::from_yaml_str(&raw).unwrap_err();
        assert!(err.to_string().contains("yaml") || err.to_string().contains("config"));
    }

    #[test]
    fn unknown_algorithm_name_is_unsupported() {
        let raw = TINY.replace(
            "modelRefs:",
            "algorithm: not-a-real-algo\n          modelRefs:",
        );
        assert!(RouterDocument::from_yaml_str(&raw).is_err());
    }

    #[test]
    fn decision_ext_without_endpoint_is_unsupported() {
        let raw = TINY.replace(
            "strategy: priority\n",
            "strategy: priority\n      decision_ext:\n        fallback: local/general\n",
        );
        // from_yaml_str validates, so a decision_ext without an endpoint is rejected.
        assert!(matches!(
            RouterDocument::from_yaml_str(&raw),
            Err(RouterError::Unsupported(_))
        ));
    }

    #[test]
    fn decision_ext_with_endpoint_validates() {
        let raw = TINY.replace(
            "strategy: priority\n",
            "strategy: priority\n      decision_ext:\n        endpoint: http://127.0.0.1:9\n",
        );
        let doc = RouterDocument::from_yaml_str(&raw).unwrap();
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn agent_delegation_fields_parse_and_validate() {
        let raw = include_str!("../examples/agent-tiny.yaml").replace(
            "fallback: local/general",
            "fallback: local/general\n      delegate_endpoint: http://127.0.0.1:9\n      worker_plan: wp-1\n      delegate_auth: tok",
        );
        let doc = RouterDocument::from_yaml_str(&raw).unwrap();
        assert!(doc.validate().is_ok());
        let a = doc.recipe("agent-default").unwrap().agent.as_ref().unwrap();
        assert_eq!(a.delegate_endpoint.as_deref(), Some("http://127.0.0.1:9"));
        assert_eq!(a.worker_plan.as_deref(), Some("wp-1"));
        assert_eq!(a.delegate_auth.as_deref(), Some("tok"));
    }

    #[test]
    fn default_config_path_under_aria_compute_home() {
        with_aria_compute_home(|home| {
            assert_eq!(default_config_path().unwrap(), home.join("router.yml"));
        });
    }

    #[test]
    fn write_semantic_template_roundtrip() {
        with_aria_compute_home(|home| {
            let path = write_default_config("semantic", true).unwrap();
            assert_eq!(path, home.join("router.yml"));
            let doc = RouterDocument::load_path(&path).unwrap();
            assert_eq!(doc.entrypoints[0].router, RouterKind::Semantic);
            let (want_s, want_m, _) = template_provider_model_ids("semantic").unwrap();
            let small = doc.provider("ariacompute/ariamodel-small").unwrap();
            assert_eq!(small.provider_model_id, want_s);
            assert_eq!(small.tier.as_deref(), Some("small"));
            let mid = doc.provider("ariacompute/ariamodel-mid").unwrap();
            assert_eq!(mid.provider_model_id, want_m);
            assert_eq!(mid.tier.as_deref(), Some("mid"));
            assert!(mid.backend_refs[0]
                .api_key_env
                .as_deref()
                .is_some_and(|e| e == "GATEWAY_API_KEY"));
            assert!(write_default_config("semantic", false).is_err());
            write_default_config("agent", true).unwrap();
            let doc = RouterDocument::load_path(&path).unwrap();
            assert_eq!(doc.entrypoints[0].router, RouterKind::Agent);
            assert!(doc
                .recipe("agent-default")
                .unwrap()
                .agent
                .as_ref()
                .unwrap()
                .endpoint
                .as_deref()
                .is_some_and(|e| e.contains("tokenhub")));
            clear_default_config().unwrap();
            assert!(!path.exists());
        });
    }

    #[test]
    fn template_provider_model_ids_match_embedded_gateway() {
        for kind in ["", "semantic", "agent", "afm-d"] {
            let (s, m, l) = template_provider_model_ids(kind).unwrap();
            let doc = RouterDocument::from_yaml_str(gateway_template_yaml(kind).unwrap()).unwrap();
            assert_eq!(
                s,
                doc.provider("ariacompute/ariamodel-small")
                    .unwrap()
                    .provider_model_id,
                "kind={kind}"
            );
            assert_eq!(
                m,
                doc.provider("ariacompute/ariamodel-mid")
                    .unwrap()
                    .provider_model_id,
                "kind={kind}"
            );
            assert_eq!(
                l,
                doc.provider("ariacompute/ariamodel-large")
                    .unwrap()
                    .provider_model_id,
                "kind={kind}"
            );
            assert!(!s.is_empty() && !m.is_empty() && !l.is_empty());
        }
        assert!(gateway_template_yaml("nope").is_err());
        assert!(template_provider_model_ids("nope").is_err());
    }

    fn assert_track_e_routing(routing: &Routing) {
        let prio = |n: &str| {
            routing
                .decisions
                .iter()
                .find(|d| d.name == n)
                .map(|d| d.priority)
                .unwrap_or_else(|| panic!("missing decision {n}"))
        };
        // Track E: multi_question > systems_mid > factoid > intro_teach
        assert!(prio("multi_question") > prio("systems_mid"));
        assert!(prio("systems_mid") > prio("factoid"));
        assert!(prio("factoid") > prio("intro_teach"));
        let needs_intro = routing
            .signals
            .keywords
            .iter()
            .find(|k| k.name == "needs_intro")
            .expect("needs_intro");
        assert!(
            !needs_intro
                .keywords
                .iter()
                .any(|w| w == "what is" || w == "what are"),
            "Track E: bare what-is must not be in needs_intro"
        );
        let needs_mid = routing
            .signals
            .keywords
            .iter()
            .find(|k| k.name == "needs_mid")
            .expect("needs_mid");
        assert!(needs_mid.keywords.iter().any(|w| w == "when to use"));
    }

    #[test]
    fn triad_gateway_has_three_entrypoints_and_track_e_priorities() {
        let doc =
            RouterDocument::from_yaml_str(include_str!("../examples/triad-gateway.yaml")).unwrap();
        assert_eq!(doc.entrypoints.len(), 3);
        let routers: Vec<_> = doc.entrypoints.iter().map(|e| e.router).collect();
        assert!(routers.contains(&RouterKind::Semantic));
        assert!(routers.contains(&RouterKind::Agent));
        assert!(routers.contains(&RouterKind::AfmD));
        let mom = doc.recipe("mom").unwrap();
        assert_track_e_routing(mom.routing.as_ref().expect("mom routing"));
        let afm = doc.recipe("afm-d-default").unwrap().afm_d.as_ref().unwrap();
        assert_eq!(afm.track, Some(AfmDTrack::Encoder));
    }

    #[test]
    fn gateway_yamls_share_track_e_keyword_priorities() {
        for (label, raw, recipe) in [
            (
                "semantic-gateway",
                include_str!("../examples/semantic-gateway.yaml"),
                "mom",
            ),
            (
                "triad-gateway",
                include_str!("../examples/triad-gateway.yaml"),
                "mom",
            ),
        ] {
            let doc = RouterDocument::from_yaml_str(raw).unwrap();
            let r = doc
                .recipe(recipe)
                .unwrap_or_else(|_| panic!("{label}: recipe {recipe}"));
            let routing = r
                .routing
                .as_ref()
                .unwrap_or_else(|| panic!("{label}: routing"));
            assert_track_e_routing(routing);
        }
    }

    #[test]
    fn write_afm_d_setup_overrides_endpoint_and_fallback() {
        with_aria_compute_home(|_| {
            let opts = SetupModelOpts {
                base_url: Some("https://example.test".into()),
                api_key_env: None,
                api_key: None,
                small_provider_model_id: None,
                mid_provider_model_id: None,
                large_provider_model_id: None,
                agent_endpoint: None,
                agent_model: None,
                agent_fallback: None,
                afm_d_endpoint: Some("http://127.0.0.1:8010".into()),
                afm_d_fallback: Some("ariacompute/ariamodel-small".into()),
            };
            let path = write_default_config_with_opts("afm-d", true, true, true, &opts).unwrap();
            let doc = RouterDocument::load_path(&path).unwrap();
            assert_eq!(doc.entrypoints[0].router, RouterKind::AfmD);
            let afm = doc.recipe("afm-d-default").unwrap().afm_d.as_ref().unwrap();
            assert_eq!(afm.endpoint.as_deref(), Some("http://127.0.0.1:8010"));
            assert_eq!(afm.fallback.as_deref(), Some("ariacompute/ariamodel-small"));
            clear_default_config().unwrap();
        });
    }

    #[test]
    fn write_setup_model_opts_override_upstream() {
        with_aria_compute_home(|_| {
            let opts = SetupModelOpts {
                base_url: Some("https://example.test".into()),
                api_key_env: Some("MY_GATEWAY_KEY".into()),
                api_key: Some("sk-test-secret".into()),
                small_provider_model_id: Some("foo-small".into()),
                mid_provider_model_id: Some("foo-mid".into()),
                large_provider_model_id: Some("foo-large".into()),
                agent_endpoint: None,
                agent_model: Some("ariacompute/ariamodel-mid".into()),
                agent_fallback: Some("ariacompute/ariamodel-small".into()),
                afm_d_endpoint: None,
                afm_d_fallback: None,
            };
            let path = write_default_config_with_opts("agent", true, true, true, &opts).unwrap();
            let doc = RouterDocument::load_path(&path).unwrap();
            assert_eq!(
                doc.provider("ariacompute/ariamodel-small")
                    .unwrap()
                    .provider_model_id,
                "foo-small"
            );
            assert_eq!(
                doc.provider("ariacompute/ariamodel-mid")
                    .unwrap()
                    .provider_model_id,
                "foo-mid"
            );
            assert_eq!(
                doc.provider("ariacompute/ariamodel-large")
                    .unwrap()
                    .provider_model_id,
                "foo-large"
            );
            let small = doc.provider("ariacompute/ariamodel-small").unwrap();
            assert_eq!(small.tier.as_deref(), Some("small"));
            assert_eq!(small.backend_refs[0].base_url, "https://example.test");
            assert_eq!(
                small.backend_refs[0].api_key_env.as_deref(),
                Some("MY_GATEWAY_KEY")
            );
            assert_eq!(
                small.backend_refs[0].api_key.as_deref(),
                Some("sk-test-secret")
            );
            let agent = doc.recipe("agent-default").unwrap().agent.as_ref().unwrap();
            assert_eq!(agent.endpoint.as_deref(), Some("https://example.test"));
            assert_eq!(agent.model.as_deref(), Some("ariacompute/ariamodel-mid"));
            assert_eq!(
                agent.fallback.as_deref(),
                Some("ariacompute/ariamodel-small")
            );
            clear_default_config().unwrap();
        });
    }

    #[test]
    fn example_yamls_validate() {
        for raw in [
            include_str!("../examples/semantic-tiny.yaml"),
            include_str!("../examples/agent-tiny.yaml"),
            include_str!("../examples/semantic.yaml"),
            include_str!("../examples/agent.yaml"),
            include_str!("../examples/semantic-gateway.yaml"),
            include_str!("../examples/agent-gateway.yaml"),
            include_str!("../examples/afm-d-tiny.yaml"),
            include_str!("../examples/afm-d.yaml"),
            include_str!("../examples/afm-d-gateway.yaml"),
            include_str!("../examples/triad-gateway.yaml"),
            include_str!("../examples/semantic-enhancements.yaml"),
        ] {
            RouterDocument::from_yaml_str(raw).unwrap();
        }
    }

    #[test]
    fn afm_d_tiny_entrypoint() {
        let doc =
            RouterDocument::from_yaml_str(include_str!("../examples/afm-d-tiny.yaml")).unwrap();
        assert_eq!(doc.entrypoints[0].router, RouterKind::AfmD);
        assert!(doc.recipe("afm-d-default").unwrap().afm_d.is_some());
    }

    #[test]
    fn afm_d_gateway_endpoint_quoted_default() {
        std::env::remove_var("DECISION_MODEL_URL");
        let doc =
            RouterDocument::from_yaml_str(include_str!("../examples/afm-d-gateway.yaml")).unwrap();
        let afm = doc.recipes[0].afm_d.as_ref().unwrap();
        assert_eq!(afm.endpoint.as_deref(), Some("http://127.0.0.1:8010"));
        assert_eq!(afm.track, Some(AfmDTrack::Encoder));
    }

    #[test]
    fn afm_d_track_decoder_rejects_too_many_text_providers() {
        let mut models = String::new();
        for i in 0..17 {
            models.push_str(&format!(
                "    - name: local/m{i}\n      modality: text\n      backend_refs: [{{name: p, endpoint: 127.0.0.1:1}}]\n"
            ));
        }
        let raw = format!(
            r#"
version: v0.3
providers:
  models:
{models}
entrypoints:
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default
recipes:
  - name: afm-d-default
    router: afm-d
    afm-d:
      track: decoder
      endpoint: http://127.0.0.1:8011
"#
        );
        let err = RouterDocument::from_yaml_str(&raw).unwrap_err().to_string();
        assert!(err.contains("track=decoder"), "{err}");
        assert!(err.contains("at most 16"), "{err}");
    }

    #[test]
    fn afm_d_track_invalid_value_rejected() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/a
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
    - name: local/b
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default
recipes:
  - name: afm-d-default
    router: afm-d
    afm-d:
      track: weird
      endpoint: http://127.0.0.1:8011
"#;
        assert!(RouterDocument::from_yaml_str(raw).is_err());
    }

    #[test]
    fn afm_d_catalog_example() {
        let doc = RouterDocument::from_yaml_str(include_str!("../examples/afm-d.yaml")).unwrap();
        assert_eq!(doc.entrypoints[0].router, RouterKind::AfmD);
        assert_eq!(
            doc.entrypoints[1].model_names,
            vec!["ariacompute/afm-d-catalog"]
        );
        assert_eq!(
            doc.recipe("afm-d-catalog")
                .unwrap()
                .afm_d
                .as_ref()
                .unwrap()
                .fallback
                .as_deref(),
            Some("local/general")
        );
    }

    #[test]
    fn catalog_examples_reference_learned_signals() {
        let semantic =
            RouterDocument::from_yaml_str(include_str!("../examples/semantic.yaml")).unwrap();
        let catalog = semantic.recipe("mom-catalog").unwrap();
        assert!(!semantic.learned_signal_referenced(catalog).is_empty());
        let mom = semantic.recipe("mom").unwrap();
        assert!(semantic.learned_signal_referenced(mom).is_empty());

        let agent =
            RouterDocument::from_yaml_str(include_str!("../examples/agent-tiny.yaml")).unwrap();
        assert_eq!(agent.entrypoints[0].router, RouterKind::Agent);
        assert_eq!(
            agent
                .recipe("agent-default")
                .unwrap()
                .agent
                .as_ref()
                .unwrap()
                .max_turns,
            Some(3)
        );
    }

    #[test]
    fn extensions_block_rejected() {
        let raw = format!(
            "{}\nextensions:\n  - name: builtin\n    type: builtin\n",
            include_str!("../examples/semantic-tiny.yaml")
        );
        assert!(RouterDocument::from_yaml_str(&raw).is_err());
    }

    #[test]
    fn retention_drop_and_ttl_conflict() {
        let raw = r#"
version: v0.3
providers:
  models:
    - name: local/general
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      decisions:
        - name: d
          rules: { operator: AND, conditions: [] }
          modelRefs: [{model: local/general}]
          emits:
            - kind: retention
              retention: { drop: true, ttl_turns: 2 }
"#;
        let err = RouterDocument::from_yaml_str(raw).unwrap_err();
        assert!(matches!(err, RouterError::Config(_)));
    }

    #[test]
    fn semantic_stateful_example_loads() {
        let doc = RouterDocument::from_yaml_str(include_str!("../examples/semantic-stateful.yaml"))
            .unwrap();
        assert!(doc.global.stores.memory.enabled);
        let d = &doc
            .recipe("mom")
            .unwrap()
            .routing
            .as_ref()
            .unwrap()
            .decisions[0];
        assert!(d.retention().is_some());
    }

    #[test]
    fn cli_config_defaults_empty_upgrade_url() {
        let cfg = RouterCliConfig::default();
        assert!(cfg.upgrade_url.is_empty());
        assert_eq!(default_upgrade_url_for_site("com"), DEFAULT_UPGRADE_URL_COM);
        assert_eq!(default_upgrade_url_for_site("cn"), DEFAULT_UPGRADE_URL_CN);
        assert_eq!(default_upgrade_url_for_site("COM"), DEFAULT_UPGRADE_URL_COM);
    }

    #[test]
    fn cli_config_roundtrip() {
        with_aria_compute_home(|_| {
            let cfg = RouterCliConfig {
                upgrade_url: DEFAULT_UPGRADE_URL_COM.to_string(),
            };
            let path = save_cli_config(&cfg).unwrap();
            assert!(path.ends_with("router-cli.yml"));
            let loaded = load_cli_config().unwrap();
            assert_eq!(loaded.upgrade_url, DEFAULT_UPGRADE_URL_COM);
            assert!(lib_dir().unwrap().ends_with("lib"));
            let cleared = clear_cli_config().unwrap();
            assert!(cleared.is_some());
            assert!(load_cli_config().unwrap().upgrade_url.is_empty());
            // Second clear is a no-op that returns None.
            assert!(clear_cli_config().unwrap().is_none());
        });
    }

    const MODELS_ONE: &str = r#"
providers:
  defaults: {default_model: m}
  models:
    - name: m
      provider_model_id: echo
      backend_refs: [{name: p, endpoint: 127.0.0.1:9}]
"#;

    fn doc_with(entrypoints: &str, recipes: &str) -> String {
        format!("version: v0.3\n{MODELS_ONE}\nentrypoints:\n{entrypoints}\nrecipes:\n{recipes}\n")
    }

    fn expect_config_err(raw: &str, needle: &str) {
        let err = RouterDocument::from_yaml_str(raw).unwrap_err();
        assert!(matches!(err, RouterError::Config(_)), "{err}");
        assert!(err.to_string().contains(needle), "{err}");
    }

    fn expect_unsupported(raw: &str, needle: &str) {
        let err = RouterDocument::from_yaml_str(raw).unwrap_err();
        assert!(matches!(err, RouterError::Unsupported(_)), "{err}");
        assert!(err.to_string().contains(needle), "{err}");
    }

    #[test]
    fn serde_defaults_catalog_listener_replay_and_pricing() {
        let doc = RouterDocument::from_yaml_str(&format!(
            r#"
version: v0.3
listeners:
  - {{}}
{MODELS_ONE}
entrypoints:
  - {{model_names: [auto], router: semantic, recipe: r}}
recipes:
  - {{name: r, router: semantic, routing: {{decisions: []}}}}
global:
  model_catalog:
    flash: {{}}
  services:
    router_replay: {{}}
"#
        ))
        .unwrap();
        assert_eq!(doc.listeners[0].name, "http");
        assert_eq!(doc.listeners[0].address, "0.0.0.0");
        assert_eq!(doc.listeners[0].port, 8899);
        assert_eq!(doc.data_bind(), "0.0.0.0:8899");
        assert_eq!(doc.global.model_catalog["flash"].kind, "hash");
        assert!(doc.global.services.router_replay.enabled);
        assert_eq!(doc.global.services.router_replay.max_items, 256);
        assert!(doc.global.require_api_key);
        assert!(doc.global.allow_register);
        assert_eq!(doc.global.serve_site, "com");

        // No listeners → default bind string.
        let raw = format!("version: v0.3\n{MODELS_ONE}\nentrypoints: []\nrecipes: []\n");
        let empty = RouterDocument::from_yaml_str(&raw).unwrap();
        assert_eq!(empty.data_bind(), "0.0.0.0:8899");
        assert!(empty
            .learned_signal_referenced(&Recipe {
                name: "x".into(),
                router: RouterKind::Semantic,
                routing: None,
                agent: None,
                afm_d: None,
            })
            .is_empty());
    }

    #[test]
    fn pricing_backend_urls_and_ranking_cost() {
        let doc = RouterDocument::from_yaml_str(&format!(
            r#"
version: v0.3
{MODELS_ONE}
entrypoints: []
recipes: []
"#
        ))
        .unwrap();
        let mut model = doc.provider("m").unwrap().clone();
        assert_eq!(model.ranking_cost(), 1.0);
        model.pricing = Some(ModelPricing {
            input_per_mtok: 1.5,
            output_per_mtok: 3.0,
        });
        assert_eq!(model.ranking_cost(), 3.0);
        model.pricing = Some(ModelPricing {
            input_per_mtok: 2.5,
            output_per_mtok: 0.0,
        });
        assert_eq!(model.ranking_cost(), 2.5);

        let mut reference = model.backend_refs[0].clone();
        assert_eq!(reference.url(), "http://127.0.0.1:9");
        reference.base_url = "http://base.example/".into();
        assert_eq!(reference.url(), "http://base.example");
        reference.base_url.clear();
        reference.endpoint = "https://up.example/".into();
        assert_eq!(reference.url(), "https://up.example");
        assert_eq!(reference.protocol, "http");
        assert_eq!(reference.weight, 100);
    }

    #[test]
    fn json_parsing_and_io_errors() {
        let json = r#"{
            "version": "v0.3",
            "providers": {"defaults": {"default_model": "m"}, "models": []},
            "entrypoints": [],
            "recipes": []
        }"#;
        let doc = RouterDocument::from_json_str(json).unwrap();
        assert_eq!(doc.version, "v0.3");
        assert!(doc.to_yaml().unwrap().contains("v0.3"));
        assert!(matches!(
            RouterDocument::from_json_str("{not json"),
            Err(RouterError::Config(_))
        ));
        assert!(matches!(
            RouterDocument::load_path("/nonexistent/router.yml"),
            Err(RouterError::Io(_))
        ));
        expect_config_err("version: v0.2\n", "version must be v0.3");
    }

    #[test]
    fn entrypoint_validation_errors() {
        expect_config_err(
            &doc_with(
                "  - {model_names: [a], router: semantic, recipe: missing}",
                "  - {name: other, router: semantic, routing: {decisions: []}}",
            ),
            "not found",
        );
        expect_config_err(
            &doc_with(
                "  - {model_names: [a], router: semantic, recipe: r}",
                "  - {name: r, router: agent, agent: {max_turns: 1}}",
            ),
            "entrypoint.router",
        );
    }

    #[test]
    fn semantic_recipe_validation_errors() {
        expect_config_err(
            &doc_with(
                "  - {model_names: [a], router: semantic, recipe: r}",
                "  - {name: r, router: semantic, routing: {decisions: []}, agent: {max_turns: 1}}",
            ),
            "must not contain agent",
        );
        expect_config_err(
            &doc_with(
                "  - {model_names: [a], router: semantic, recipe: r}",
                "  - {name: r, router: semantic, routing: {decisions: []}, afm-d: {endpoint: x}}",
            ),
            "must not contain afm-d",
        );
        expect_config_err(
            &doc_with(
                "  - {model_names: [a], router: semantic, recipe: r}",
                "  - {name: r, router: semantic}",
            ),
            "missing routing",
        );
    }

    #[test]
    fn agent_recipe_validation_errors() {
        let ep = "  - {model_names: [a], router: agent, recipe: r}";
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: agent, agent: {max_turns: 1}, afm-d: {endpoint: x}}",
            ),
            "must not contain afm-d",
        );
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: agent, agent: {max_turns: 1}, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}]}]}}",
            ),
            "must not contain signals/decisions",
        );
        expect_config_err(
            &doc_with(ep, "  - {name: r, router: agent}"),
            "missing agent",
        );
        expect_config_err(
            &doc_with(ep, "  - {name: r, router: agent, agent: {fallback: ghost}}"),
            "fallback ghost not in providers",
        );
        expect_config_err(
            &doc_with(ep, "  - {name: r, router: agent, agent: {max_turns: 0}}"),
            "max_turns must be 1..=8",
        );
        expect_config_err(
            &doc_with(ep, "  - {name: r, router: agent, agent: {max_turns: 9}}"),
            "max_turns must be 1..=8",
        );
    }

    #[test]
    fn afm_d_recipe_validation_errors() {
        let ep = "  - {model_names: [a], router: afm-d, recipe: r}";
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: afm-d, afm-d: {endpoint: x}, agent: {max_turns: 1}}",
            ),
            "must not contain agent",
        );
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: afm-d, afm-d: {endpoint: x}, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}]}]}}",
            ),
            "must not contain signals/decisions",
        );
        expect_config_err(
            &doc_with(ep, "  - {name: r, router: afm-d}"),
            "missing afm-d",
        );
        expect_config_err(
            &doc_with(ep, "  - {name: r, router: afm-d, afm-d: {fallback: ghost}}"),
            "fallback ghost not in providers",
        );
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: afm-d, afm-d: {min_confidence: 2.0}}",
            ),
            "min_confidence must be 0..=1",
        );

        // Track ceiling: decoder allows 16 text providers.
        let mut models = String::from("providers:\n  models:\n");
        for i in 0..17 {
            models.push_str(&format!(
                "    - name: m{i}\n      backend_refs: [{{name: p, endpoint: 127.0.0.1:9}}]\n"
            ));
        }
        let raw = format!(
            "version: v0.3\n{models}entrypoints:\n{ep}\nrecipes:\n  - {{name: r, router: afm-d, afm-d: {{track: decoder}}}}\n"
        );
        expect_config_err(&raw, "allows at most 16 text providers");

        // Encoder ceiling is much higher, so the same pool is accepted.
        let raw = raw.replace("track: decoder", "track: encoder");
        let doc = RouterDocument::from_yaml_str(&raw).unwrap();
        assert_eq!(
            doc.recipes[0].afm_d.as_ref().unwrap().track,
            Some(AfmDTrack::Encoder)
        );
    }

    #[test]
    fn routing_validation_errors() {
        let ep = "  - {model_names: [a], router: semantic, recipe: r}";
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [], signals: {weird: {}}}}",
            ),
            "unknown signal family",
        );
        // An empty signal-family key is ignored instead of rejected: it carries no
        // meaning and YAML maps can produce it from templated configs.
        let doc = RouterDocument::from_yaml_str(&doc_with(
            ep,
            "  - {name: r, router: semantic, routing: {decisions: [], signals: {\"\": {}}}}",
        ))
        .unwrap();
        assert_eq!(doc.recipes.len(), 1);
        expect_unsupported(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}], algorithm: mystery}]}}",
            ),
            "unknown algorithm",
        );
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}], emits: [{kind: retention}]}]}}",
            ),
            "missing retention block",
        );
        expect_unsupported(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}], emits: [{kind: telemetry}]}]}}",
            ),
            "unknown emit kind",
        );
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}], emits: [{kind: retention, retention: {drop: true, ttl_turns: 2}}]}]}}",
            ),
            "conflicts with positive ttl_turns",
        );
        expect_config_err(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [{name: d, rules: {operator: AND, conditions: []}, modelRefs: [{model: m}], emits: [{kind: retention, retention: {ttl_turns: 1}}, {kind: retention, retention: {ttl_turns: 2}}]}]}}",
            ),
            "duplicate emit retention",
        );
        expect_unsupported(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [], decision_ext: {track: encoder}}}",
            ),
            "requires a non-empty `endpoint`",
        );
        expect_unsupported(
            &doc_with(
                ep,
                "  - {name: r, router: semantic, routing: {decisions: [], decision_ext: {endpoint: \"  \"}}}",
            ),
            "requires a non-empty `endpoint`",
        );
    }

    #[test]
    fn retention_lookup_ignores_non_retention_emits() {
        let decision = DecisionCfg {
            name: "d".into(),
            description: None,
            priority: 1,
            rules: RuleNode::default(),
            model_refs: vec![ModelRef { model: "m".into() }],
            algorithm: None,
            plugins: vec![],
            locality: None,
            emits: vec![EmitDirective {
                kind: "other".into(),
                retention: Some(RetentionDirective::default()),
            }],
        };
        assert!(decision.retention().is_none());
        assert_eq!(AfmDTrack::Encoder.to_string(), "encoder");
        assert_eq!(AfmDTrack::Decoder.to_string(), "decoder");
    }

    #[test]
    fn patch_setup_models_ignores_unexpected_shapes() {
        let opts = SetupModelOpts {
            base_url: Some("http://x".into()),
            api_key_env: Some("KEY".into()),
            api_key: Some("sk".into()),
            small_provider_model_id: Some("s".into()),
            mid_provider_model_id: Some("m".into()),
            large_provider_model_id: Some("l".into()),
            agent_endpoint: Some("http://agent".into()),
            agent_model: Some("am".into()),
            agent_fallback: Some("ariacompute/ariamodel-small".into()),
            afm_d_endpoint: Some("http://afm".into()),
            afm_d_fallback: Some("ariacompute/ariamodel-large".into()),
        };

        // Non-mapping root is a no-op.
        let mut scalar = serde_yaml::Value::String("nope".into());
        patch_setup_models(&mut scalar, &opts);
        assert_eq!(scalar, serde_yaml::Value::String("nope".into()));

        // Odd shapes inside models/refs/recipes are skipped, unknown model names untouched.
        let raw = r#"
providers:
  models:
    - just-a-string
    - name: ariacompute/ariamodel-small
      backend_refs:
        - not-a-mapping
        - name: p
    - name: unknown-model
recipes:
  - just-a-string
  - name: agent-recipe
    router: agent
  - name: afm-recipe
    router: afm-d
"#;
        let mut doc: serde_yaml::Value = serde_yaml::from_str(raw).unwrap();
        patch_setup_models(&mut doc, &opts);
        let models = doc
            .get("providers")
            .and_then(|p| p.get("models"))
            .and_then(|m| m.as_sequence())
            .unwrap();
        assert_eq!(
            models[1]["provider_model_id"],
            serde_yaml::Value::String("s".into())
        );
        assert_eq!(
            models[1]["backend_refs"][1]["base_url"],
            serde_yaml::Value::String("http://x".into())
        );
        assert!(models[2].get("provider_model_id").is_none());

        // A document without a `recipes` sequence stops after patching models.
        let mut no_recipes: serde_yaml::Value =
            serde_yaml::from_str("providers:\n  models:\n    - name: ariacompute/ariamodel-mid\n")
                .unwrap();
        patch_setup_models(&mut no_recipes, &opts);
        let mid = &no_recipes["providers"]["models"][0];
        assert_eq!(
            mid["provider_model_id"],
            serde_yaml::Value::String("m".into())
        );

        // No agent/afm-d overrides → early return after models.
        let mut doc_only_models: serde_yaml::Value =
            serde_yaml::from_str("providers:\n  models: []\n").unwrap();
        patch_setup_models(&mut doc_only_models, &SetupModelOpts::default());
        assert!(doc_only_models.get("recipes").is_none());
    }

    #[test]
    fn write_and_clear_default_config_roundtrip() {
        with_aria_compute_home(|dir| {
            let path = write_default_config("semantic", true).unwrap();
            assert!(path.exists());
            assert!(dir.join("router-keys.json").exists());
            assert!(dir.join("router-users.json").exists());
            // Refuses to clobber without overwrite.
            let err = write_default_config("semantic", false).unwrap_err();
            assert!(matches!(err, RouterError::Io(_)));
            // Unknown template kind is invalid.
            assert!(matches!(
                write_default_config("bogus", true),
                Err(RouterError::InvalidParam(_))
            ));
            // Agent + afm-d templates with overrides are written and self-validate.
            let opts = SetupModelOpts {
                base_url: Some("http://upstream:1".into()),
                api_key_env: Some("GATEWAY_API_KEY".into()),
                api_key: Some("sk-test".into()),
                small_provider_model_id: Some("s".into()),
                mid_provider_model_id: Some("m".into()),
                large_provider_model_id: Some("l".into()),
                agent_endpoint: Some("http://agent:2".into()),
                agent_model: Some("am".into()),
                agent_fallback: Some("ariacompute/ariamodel-small".into()),
                afm_d_endpoint: Some("http://afm:3".into()),
                afm_d_fallback: Some("ariacompute/ariamodel-large".into()),
            };
            write_default_config_with_opts("agent", true, true, false, &opts).unwrap();
            write_default_config_with_opts("afm-d", true, false, true, &opts).unwrap();
            let raw = std::fs::read_to_string(default_config_path().unwrap()).unwrap();
            assert!(raw.contains("http://afm:3"));
            assert!(raw.contains("allow_register: true"));

            let cleared = clear_default_config().unwrap();
            assert!(!cleared.exists());
            // Clearing again is still Ok.
            assert!(!clear_default_config().unwrap().exists());
        });
    }

    #[test]
    fn home_and_path_resolution() {
        with_aria_compute_home(|dir| {
            assert_eq!(aria_home().unwrap(), dir);
            assert!(default_config_path().unwrap().ends_with("router.yml"));
            assert!(default_keys_path().unwrap().ends_with("router-keys.json"));
            assert!(default_users_path().unwrap().ends_with("router-users.json"));
            assert!(default_cli_config_path()
                .unwrap()
                .ends_with("router-cli.yml"));
            assert!(ensure_aria_home().unwrap().join("tmp").is_dir());
            // Empty override falls back to `$HOME/.ariacompute`.
            assert_eq!(resolve_keys_path("").unwrap(), default_keys_path().unwrap());
            assert_eq!(
                resolve_users_path(" ").unwrap(),
                default_users_path().unwrap()
            );
            assert_eq!(
                resolve_keys_path("/abs/keys.json").unwrap(),
                PathBuf::from("/abs/keys.json")
            );
            assert_eq!(
                resolve_keys_path("rel/keys.json").unwrap(),
                dir.join("rel/keys.json")
            );
            let home_relative = resolve_keys_path("~/keys.json").unwrap();
            assert!(home_relative.ends_with("keys.json"));
            assert!(!home_relative.starts_with(dir));
        });

        // Empty ARIA_COMPUTE_HOME is treated as unset.
        {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
            std::env::set_var("ARIA_COMPUTE_HOME", "");
            let home = aria_home().unwrap();
            assert!(home.ends_with(".ariacompute"));
            restore_env(prev);
        }
    }

    #[test]
    fn template_lookup_and_provider_ids() {
        assert!(gateway_template_yaml("").unwrap().contains("version: v0.3"));
        assert!(gateway_template_yaml("semantic").is_ok());
        assert!(gateway_template_yaml("agent")
            .unwrap()
            .contains("router: agent"));
        assert!(gateway_template_yaml("afm-d")
            .unwrap()
            .contains("router: afm-d"));
        assert!(matches!(
            gateway_template_yaml("bogus"),
            Err(RouterError::InvalidParam(_))
        ));
        for kind in ["semantic", "agent", "afm-d"] {
            let (small, mid, large) = template_provider_model_ids(kind).unwrap();
            assert!(!small.is_empty());
            assert!(!mid.is_empty());
            assert!(!large.is_empty());
        }
        // A template without upstream ids fails closed with a config error.
        let doc = RouterDocument::from_yaml_str(
            "version: v0.3\nproviders:\n  models:\n    - name: ariacompute/ariamodel-small\n",
        )
        .unwrap();
        let err = provider_model_ids_from(&doc, "test").unwrap_err();
        assert!(matches!(err, RouterError::Config(_)), "{err}");
        assert!(
            err.to_string().contains("missing provider_model_id"),
            "{err}"
        );
    }

    #[test]
    fn env_helper_propagates_panic() {
        // `resume_unwind` path of the env helper.
        let res = std::panic::catch_unwind(|| with_aria_compute_home(|_| panic!("boom")));
        assert!(res.is_err());
    }

    #[test]
    fn env_helper_restores_previous_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        with_aria_compute_home_locked(Some("/tmp/aria-prev-home".into()), |dir| {
            assert!(dir.exists());
        });
        assert_eq!(
            std::env::var("ARIA_COMPUTE_HOME").unwrap(),
            "/tmp/aria-prev-home"
        );
        std::env::remove_var("ARIA_COMPUTE_HOME");
    }

    #[test]
    fn aria_home_falls_back_to_home_dir_without_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
        std::env::remove_var("ARIA_COMPUTE_HOME");
        let resolved = aria_home();
        restore_env(prev);
        let home = resolved.expect("home dir resolves on supported platforms");
        assert!(home.ends_with(".ariacompute"), "{}", home.display());
    }

    #[test]
    fn validation_accepts_agent_and_afm_d_without_optional_limits() {
        // `max_turns` absent: the range check is skipped entirely.
        let agent_ep = "  - {model_names: [a], router: agent, recipe: r}";
        for recipe in [
            "  - {name: r, router: agent, agent: {}}",
            "  - {name: r, router: agent, agent: {max_turns: 2}}",
        ] {
            let doc = RouterDocument::from_yaml_str(&doc_with(agent_ep, recipe)).unwrap();
            assert_eq!(doc.recipes.len(), 1);
        }

        // A `min_confidence` inside 0..=1 passes the range check.
        let afm_ep = "  - {model_names: [a], router: afm-d, recipe: r}";
        let doc = RouterDocument::from_yaml_str(&doc_with(
            afm_ep,
            "  - {name: r, router: afm-d, afm-d: {min_confidence: 0.5}}",
        ))
        .unwrap();
        assert_eq!(doc.recipes.len(), 1);
    }

    #[test]
    fn patch_setup_models_tolerates_missing_providers_and_models() {
        let opts = SetupModelOpts {
            api_key: Some("sk-x".into()),
            ..SetupModelOpts::default()
        };
        let mut no_providers: serde_yaml::Value = serde_yaml::from_str("version: v0.3\n").unwrap();
        patch_setup_models(&mut no_providers, &opts);
        assert!(no_providers.get("providers").is_none());

        let mut no_models: serde_yaml::Value =
            serde_yaml::from_str("providers:\n  defaults: {}\n").unwrap();
        patch_setup_models(&mut no_models, &opts);
        assert!(no_models["providers"].get("models").is_none());
    }
}
