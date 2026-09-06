//! Heuristic (and gated learned) signal extraction.

use aria_router_config::{Recipe, RouterDocument, Signals};
use aria_router_core::{ChatRequest, RouterError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalHit {
    pub kind: String,
    pub name: String,
    pub matched: bool,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SignalSet {
    pub hits: Vec<SignalHit>,
}

impl SignalSet {
    pub fn get(&self, kind: &str, name: &str) -> Option<&SignalHit> {
        self.hits
            .iter()
            .find(|h| h.kind == kind && h.name == name)
    }

    pub fn matched(&self, kind: &str, name: &str) -> bool {
        self.get(kind, name).is_some_and(|h| h.matched)
    }
}

const LEARNED: &[&str] = &[
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

pub fn extract(
    doc: &RouterDocument,
    recipe: &Recipe,
    req: &ChatRequest,
    metadata: &HashMap<String, String>,
) -> Result<SignalSet, RouterError> {
    let Some(routing) = &recipe.routing else {
        return Ok(SignalSet::default());
    };
    let needed = referenced(&routing.decisions);
    if let Some(kind) = needed.iter().find(|k| LEARNED.contains(&k.as_str())) {
        return Err(RouterError::Unsupported(format!(
            "learned signal {kind} requires feature ml / weights"
        )));
    }
    let prompt = req.prompt_text();
    let mut hits = vec![];
    let s = &routing.signals;
    if needed.iter().any(|k| k == "keyword") {
        hits.extend(eval_keywords(s, &prompt));
    }
    if needed.iter().any(|k| k == "language") {
        hits.extend(eval_language(s, &prompt));
    }
    if needed.iter().any(|k| k == "context") {
        hits.extend(eval_context(s, &prompt));
    }
    if needed.iter().any(|k| k == "authz") {
        hits.extend(eval_authz(s, metadata));
    }
    if needed.iter().any(|k| k == "conversation") {
        hits.extend(eval_conversation(s, req));
    }
    if needed.iter().any(|k| k == "metadata") {
        hits.extend(eval_metadata(s, metadata));
    }
    if needed.iter().any(|k| k == "event") {
        hits.extend(eval_event(s, &prompt));
    }
    if needed.iter().any(|k| k == "structure") {
        hits.extend(eval_structure(s, &prompt));
    }
    let _ = doc;
    Ok(SignalSet { hits })
}

fn referenced(decisions: &[aria_router_config::DecisionCfg]) -> Vec<String> {
    let mut kinds = vec![];
    for d in decisions {
        collect_kinds(&d.rules, &mut kinds);
    }
    kinds.sort();
    kinds.dedup();
    kinds
}

fn collect_kinds(node: &aria_router_config::RuleNode, out: &mut Vec<String>) {
    for c in &node.conditions {
        out.push(c.kind.clone());
    }
    if let Some(inner) = &node.not {
        collect_kinds(inner, out);
    }
}

fn eval_keywords(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    let lower = prompt.to_ascii_lowercase();
    s.keywords
        .iter()
        .map(|k| {
            let hits: Vec<bool> = k
                .keywords
                .iter()
                .map(|w| lower.contains(&w.to_ascii_lowercase()))
                .collect();
            let matched = match k.operator.to_ascii_uppercase().as_str() {
                "AND" => !hits.is_empty() && hits.iter().all(|x| *x),
                "NOR" => hits.iter().all(|x| !*x),
                _ => hits.iter().any(|x| *x),
            };
            SignalHit {
                kind: "keyword".into(),
                name: k.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_language(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    s.language
        .iter()
        .map(|n| {
            let want = n
                .extra
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("en");
            let matched = detect_lang(prompt) == want;
            SignalHit {
                kind: "language".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn detect_lang(prompt: &str) -> &'static str {
    let cjk = prompt.chars().filter(|c| {
        let u = *c as u32;
        (0x4E00..=0x9FFF).contains(&u) || (0x3040..=0x30FF).contains(&u)
    }).count();
    if cjk > prompt.chars().count() / 8 && cjk > 0 {
        "zh"
    } else {
        "en"
    }
}

fn eval_context(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    let tokens = (prompt.chars().count().div_ceil(4)) as u32;
    s.context
        .iter()
        .map(|c| {
            let ge = c.min_tokens.unwrap_or(0);
            let le = c.max_tokens.unwrap_or(u32::MAX);
            let matched = tokens >= ge && tokens <= le;
            SignalHit {
                kind: "context".into(),
                name: c.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_authz(s: &Signals, metadata: &HashMap<String, String>) -> Vec<SignalHit> {
    let role = metadata.get("role").cloned().unwrap_or_default();
    s.authz
        .iter()
        .map(|n| {
            let want = n
                .extra
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let matched = !want.is_empty() && role == want;
            SignalHit {
                kind: "authz".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_conversation(s: &Signals, req: &ChatRequest) -> Vec<SignalHit> {
    s.conversation
        .iter()
        .map(|c| {
            let min = c.min_messages.unwrap_or(1);
            let matched = req.messages.len() >= min;
            SignalHit {
                kind: "conversation".into(),
                name: c.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_metadata(s: &Signals, metadata: &HashMap<String, String>) -> Vec<SignalHit> {
    s.metadata
        .iter()
        .map(|m| {
            let matched = metadata.get(&m.key).is_some_and(|v| v == &m.value);
            SignalHit {
                kind: "metadata".into(),
                name: m.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_event(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    s.event
        .iter()
        .map(|n| {
            let needle = n
                .extra
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or(&n.name);
            let matched = prompt.contains(needle);
            SignalHit {
                kind: "event".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

fn eval_structure(s: &Signals, prompt: &str) -> Vec<SignalHit> {
    let qmarks = prompt.matches('?').count() + prompt.matches('？').count();
    s.structure
        .iter()
        .map(|n| {
            let min_q = n
                .extra
                .get("min_questions")
                .and_then(|v| v.as_u64())
                .unwrap_or(2);
            let matched = qmarks as u64 >= min_q;
            SignalHit {
                kind: "structure".into(),
                name: n.name.clone(),
                matched,
                confidence: if matched { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_config::RouterDocument;

    #[test]
    fn keyword_or() {
        let raw = include_str!("../../config/examples/semantic-tiny.yaml");
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let req = ChatRequest {
            model: "ariacompute/semantic-auto".into(),
            messages: vec![aria_router_core::ChatMessage {
                role: "user".into(),
                content: serde_json::Value::String("please explain rust".into()),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let set = extract(&doc, recipe, &req, &HashMap::new()).unwrap();
        assert!(set.matched("keyword", "needs_explain"));
    }

    #[test]
    fn learned_referenced_is_unsupported() {
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
      signals:
        keywords: []
        classifier: { name: intent }
      decisions:
        - name: d
          rules:
            operator: AND
            conditions:
              - type: classifier
                name: intent
          modelRefs: [{model: local/general}]
"#;
        let doc = RouterDocument::from_yaml_str(raw).unwrap();
        let recipe = doc.recipe("mom").unwrap();
        let req = ChatRequest {
            model: "auto".into(),
            messages: vec![],
            stream: false,
            max_tokens: None,
            temperature: None,
            extra: Default::default(),
        };
        let err = extract(&doc, recipe, &req, &HashMap::new()).unwrap_err();
        assert!(matches!(err, RouterError::Unsupported(_)));
    }
}
