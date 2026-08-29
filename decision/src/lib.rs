//! Boolean decisions + projections.

use aria_router_config::{DecisionCfg, ProjectionCfg, Recipe, RuleNode};
use aria_router_core::RouterError;
use aria_router_signal::SignalSet;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ProjectionMap {
    pub values: HashMap<String, Value>,
}

pub fn project(cfgs: &[ProjectionCfg], signals: &SignalSet) -> Result<ProjectionMap, RouterError> {
    let mut values = HashMap::new();
    for p in cfgs {
        match p.kind.as_str() {
            "partition" => {
                let inputs: Vec<String> = p
                    .extra
                    .get("signals")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let winner = inputs.into_iter().find(|name| {
                    signals.hits.iter().any(|h| h.name == *name && h.matched)
                });
                values.insert(
                    p.name.clone(),
                    winner.map(Value::String).unwrap_or(Value::Null),
                );
            }
            "score" => {
                let mut acc = 0.0;
                if let Some(weights) = p.extra.get("weights").and_then(|v| v.as_object()) {
                    for (name, w) in weights {
                        let w = w.as_f64().unwrap_or(0.0) as f32;
                        if let Some(h) = signals.hits.iter().find(|h| h.name == *name) {
                            acc += h.confidence * w;
                        }
                    }
                }
                values.insert(p.name.clone(), serde_json::json!(acc));
            }
            "mapping" => {
                let src = p.extra.get("from").and_then(|v| v.as_str()).unwrap_or("");
                let score = values.get(src).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let bands = p.extra.get("bands").and_then(|v| v.as_array());
                let mut label = "default";
                if let Some(bands) = bands {
                    for b in bands {
                        let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("band");
                        let min = b.get("min").and_then(|v| v.as_f64()).unwrap_or(f64::MIN);
                        let max = b.get("max").and_then(|v| v.as_f64()).unwrap_or(f64::MAX);
                        if score >= min && score <= max {
                            label = name;
                            break;
                        }
                    }
                }
                values.insert(p.name.clone(), Value::String(label.into()));
            }
            other => {
                return Err(RouterError::Unsupported(format!("projection type {other}")));
            }
        }
    }
    Ok(ProjectionMap { values })
}

pub fn select_decision<'a>(
    recipe: &'a Recipe,
    signals: &SignalSet,
    strategy: &str,
) -> Result<Option<&'a DecisionCfg>, RouterError> {
    let Some(routing) = &recipe.routing else {
        return Ok(None);
    };
    let mut matched: Vec<&DecisionCfg> = routing
        .decisions
        .iter()
        .filter(|d| eval_rule(&d.rules, signals).unwrap_or(false))
        .collect();
    if matched.is_empty() {
        return Ok(None);
    }
    match strategy {
        "confidence" => {
            matched.sort_by(|a, b| {
                let ca = confidence_of(a, signals);
                let cb = confidence_of(b, signals);
                cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.priority.cmp(&a.priority))
            });
        }
        _ => matched.sort_by(|a, b| b.priority.cmp(&a.priority)),
    }
    Ok(matched.into_iter().next())
}

fn confidence_of(d: &DecisionCfg, signals: &SignalSet) -> f32 {
    d.rules
        .conditions
        .iter()
        .filter_map(|c| signals.get(&c.kind, &c.name).map(|h| h.confidence))
        .fold(1.0_f32, |a, b| a.min(b))
}

pub fn eval_rule(node: &RuleNode, signals: &SignalSet) -> Result<bool, RouterError> {
    if let Some(inner) = &node.not {
        return Ok(!eval_rule(inner, signals)?);
    }
    if node.conditions.is_empty() {
        return Ok(true);
    }
    let op = node
        .operator
        .as_deref()
        .unwrap_or("AND")
        .to_ascii_uppercase();
    let vals: Vec<bool> = node
        .conditions
        .iter()
        .map(|c| signals.matched(&c.kind, &c.name))
        .collect();
    Ok(match op.as_str() {
        "OR" => vals.iter().any(|v| *v),
        "NOT" => vals.iter().all(|v| !*v),
        _ => vals.iter().all(|v| *v),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_signal::SignalHit;

    #[test]
    fn and_or() {
        let mut set = SignalSet::default();
        set.hits.push(SignalHit {
            kind: "keyword".into(),
            name: "a".into(),
            matched: true,
            confidence: 1.0,
        });
        let node = RuleNode {
            operator: Some("AND".into()),
            conditions: vec![aria_router_config::Condition {
                kind: "keyword".into(),
                name: "a".into(),
            }],
            not: None,
        };
        assert!(eval_rule(&node, &set).unwrap());
    }
}
