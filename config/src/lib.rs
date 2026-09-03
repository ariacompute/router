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
    pub extensions: Vec<ExtensionCfg>,
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
    #[serde(default)]
    pub require_api_key: bool,
    /// Allow Dashboard self-registration for local users (default true when written by setup).
    #[serde(default = "default_allow_register")]
    pub allow_register: bool,
    #[serde(default)]
    pub keys_path: Option<String>,
    #[serde(default)]
    pub users_path: Option<String>,
    #[serde(default)]
    pub serve_account_path: Option<String>,
}

impl Default for GlobalCfg {
    fn default() -> Self {
        Self {
            require_api_key: false,
            allow_register: true,
            keys_path: None,
            users_path: None,
            serve_account_path: None,
        }
    }
}

fn default_allow_register() -> bool {
    true
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
pub struct ExtensionCfg {
    pub name: String,
    #[serde(rename = "type")]
    pub ext_type: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub endpoint: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecipe {
    pub extension: String,
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
}

const LEARNED_SIGNAL_KEYS: &[&str] = &[
    "classifier",
    "complexity",
    "domain",
    "embedding",
    "fact-check",
    "fact_check",
    "jailbreak",
    "kb",
    "modality",
    "pii",
    "preference",
    "reask",
    "user-feedback",
    "user_feedback",
];

const IMPLEMENTED_ALGOS: &[&str] = &["static", "latency-aware", "latency_aware", "multi-factor", "multi_factor"];
const KNOWN_ALGOS: &[&str] = &[
    "static",
    "latency-aware",
    "latency_aware",
    "multi-factor",
    "multi_factor",
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
        let doc: Self = serde_json::from_str(raw)
            .map_err(|e| RouterError::Config(format!("json: {e}")))?;
        doc.validate()?;
        Ok(doc)
    }

    pub fn to_yaml(&self) -> Result<String, RouterError> {
        serde_yaml::to_string(self).map_err(|e| RouterError::Config(format!("yaml: {e}")))
    }

    pub fn load_path(path: impl AsRef<Path>) -> Result<Self, RouterError> {
        let raw = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            RouterError::Io(format!("read {}: {e}", path.as_ref().display()))
        })?;
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
                    let routing = recipe.routing.as_ref().ok_or_else(|| {
                        RouterError::Config(format!("semantic recipe {} missing routing", recipe.name))
                    })?;
                    self.validate_routing(routing)?;
                }
                RouterKind::Agent => {
                    if recipe.routing.as_ref().is_some_and(|r| {
                        !r.signals.keywords.is_empty() || !r.decisions.is_empty()
                    }) {
                        return Err(RouterError::Config(format!(
                            "agent recipe {} must not contain signals/decisions",
                            recipe.name
                        )));
                    }
                    if recipe.agent.is_none() {
                        return Err(RouterError::Config(format!(
                            "agent recipe {} missing agent:",
                            recipe.name
                        )));
                    }
                    let ext_name = &recipe.agent.as_ref().unwrap().extension;
                    if !self.extensions.iter().any(|e| e.name == *ext_name) {
                        return Err(RouterError::Config(format!(
                            "agent extension {ext_name} not in extensions:"
                        )));
                    }
                }
            }
        }
        for ext in &self.extensions {
            match ext.ext_type.as_str() {
                "builtin" | "pi" | "deepseek-harness" => {}
                other => {
                    return Err(RouterError::Config(format!(
                        "unknown extension type {other}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_routing(&self, routing: &Routing) -> Result<(), RouterError> {
        for (key, val) in &routing.signals.extra {
            if LEARNED_SIGNAL_KEYS.contains(&key.as_str()) {
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
            } else if !key.is_empty() {
                return Err(RouterError::Config(format!("unknown signal family {key}")));
            }
        }
        for d in &routing.decisions {
            if let Some(algo) = &d.algorithm {
                if !KNOWN_ALGOS.contains(&algo.as_str()) {
                    return Err(RouterError::Unsupported(format!("unknown algorithm {algo}")));
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
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}").expect("regex");
    re.replace_all(raw, |caps: &regex::Captures| {
        let key = &caps[1];
        let default = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        std::env::var(key).unwrap_or_else(|_| default.to_string())
    })
    .into_owned()
}

/// Embedded starter templates written by `aria-router setup`.
pub const SEMANTIC_TINY_YAML: &str = include_str!("../examples/semantic-tiny.yaml");
pub const AGENT_TINY_YAML: &str = include_str!("../examples/agent-tiny.yaml");

/// `$HOME/.ariacompute` (overridable via `ARIA_COMPUTE_HOME`).
pub fn aria_home() -> Result<PathBuf, RouterError> {
    if let Ok(override_home) = std::env::var("ARIA_COMPUTE_HOME") {
        if !override_home.is_empty() {
            return Ok(PathBuf::from(override_home));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| {
        RouterError::Io("could not resolve home directory".into())
    })?;
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

pub fn default_serve_account_path() -> Result<PathBuf, RouterError> {
    Ok(aria_home()?.join("router-serve.json"))
}

/// Expand `~/` or leave absolute/relative paths as-is under aria home resolution.
pub fn resolve_home_path(raw: &str, default: fn() -> Result<PathBuf, RouterError>) -> Result<PathBuf, RouterError> {
    let t = raw.trim();
    if t.is_empty() {
        return default();
    }
    if let Some(rest) = t.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| {
            RouterError::Io("could not resolve home directory".into())
        })?;
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

pub fn resolve_serve_account_path(raw: &str) -> Result<PathBuf, RouterError> {
    resolve_home_path(raw, default_serve_account_path)
}

/// Write a v0.3 starter YAML to `~/.ariacompute/router.yml`.
/// `kind` is `semantic` (default) or `agent`.
pub fn write_default_config(kind: &str, overwrite: bool) -> Result<PathBuf, RouterError> {
    write_default_config_with(kind, overwrite, false, true)
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
    let path = default_config_path()?;
    if path.exists() && !overwrite {
        return Err(RouterError::Io(format!(
            "{} exists (pass overwrite or --clear)",
            path.display()
        )));
    }
    let body = match kind {
        "" | "semantic" => SEMANTIC_TINY_YAML,
        "agent" => AGENT_TINY_YAML,
        other => {
            return Err(RouterError::InvalidParam(format!(
                "setup template must be semantic|agent, got {other}"
            )))
        }
    };
    let keys = default_keys_path()?;
    let users = default_users_path()?;
    let serve = default_serve_account_path()?;
    let disp = |p: &PathBuf| {
        format!(
            "~/.ariacompute/{}",
            p.file_name().and_then(|s| s.to_str()).unwrap_or("file")
        )
    };
    let mut doc: serde_yaml::Value = serde_yaml::from_str(body).map_err(|e| {
        RouterError::Config(format!("embedded template: {e}"))
    })?;
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
            serde_yaml::Value::String("serve_account_path".into()),
            serde_yaml::Value::String(disp(&serve)),
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
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
  - model_names: [aria/semantic-auto]
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
    fn default_config_path_under_aria_compute_home() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
        std::env::set_var("ARIA_COMPUTE_HOME", dir.path());
        assert_eq!(default_config_path().unwrap(), dir.path().join("router.yml"));
        match prev {
            Some(v) => std::env::set_var("ARIA_COMPUTE_HOME", v),
            None => std::env::remove_var("ARIA_COMPUTE_HOME"),
        }
    }

    #[test]
    fn write_semantic_template_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var("ARIA_COMPUTE_HOME").ok();
        std::env::set_var("ARIA_COMPUTE_HOME", dir.path());
        let path = write_default_config("semantic", true).unwrap();
        assert_eq!(path, dir.path().join("router.yml"));
        let doc = RouterDocument::load_path(&path).unwrap();
        assert_eq!(doc.entrypoints[0].router, RouterKind::Semantic);
        assert!(write_default_config("semantic", false).is_err());
        write_default_config("agent", true).unwrap();
        let doc = RouterDocument::load_path(&path).unwrap();
        assert_eq!(doc.entrypoints[0].router, RouterKind::Agent);
        clear_default_config().unwrap();
        assert!(!path.exists());
        match prev {
            Some(v) => std::env::set_var("ARIA_COMPUTE_HOME", v),
            None => std::env::remove_var("ARIA_COMPUTE_HOME"),
        }
    }

    #[test]
    fn example_yamls_validate() {
        for raw in [
            include_str!("../examples/semantic-tiny.yaml"),
            include_str!("../examples/agent-tiny.yaml"),
            include_str!("../examples/ffi-tiny.yaml"),
            include_str!("../examples/semantic.yaml"),
            include_str!("../examples/agent.yaml"),
            include_str!("../examples/ffi.yaml"),
        ] {
            RouterDocument::from_yaml_str(raw).unwrap();
        }
    }

    #[test]
    fn catalog_examples_reference_learned_and_declare_extensions() {
        let semantic = RouterDocument::from_yaml_str(include_str!("../examples/semantic.yaml")).unwrap();
        let catalog = semantic.recipe("mom-catalog").unwrap();
        assert!(!semantic.learned_signal_referenced(catalog).is_empty());
        let mom = semantic.recipe("mom").unwrap();
        assert!(semantic.learned_signal_referenced(mom).is_empty());

        let ffi = RouterDocument::from_yaml_str(include_str!("../examples/ffi.yaml")).unwrap();
        let ffi_cat = ffi.recipe("mom-catalog").unwrap();
        assert!(!ffi.learned_signal_referenced(ffi_cat).is_empty());

        let agent = RouterDocument::from_yaml_str(include_str!("../examples/agent.yaml")).unwrap();
        let types: Vec<&str> = agent.extensions.iter().map(|e| e.ext_type.as_str()).collect();
        assert!(types.contains(&"builtin"));
        assert!(types.contains(&"pi"));
        assert!(types.contains(&"deepseek-harness"));
        assert_eq!(agent.recipe("agent-pi").unwrap().agent.as_ref().unwrap().extension, "pi");
        assert_eq!(agent.recipe("agent-dsh").unwrap().agent.as_ref().unwrap().extension, "dsh");
    }
}
