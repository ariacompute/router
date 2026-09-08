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
    projections: &ProjectionMap,
    strategy: &str,
) -> Result<Option<&'a DecisionCfg>, RouterError> {
    let Some(routing) = &recipe.routing else {
        return Ok(None);
    };
    let mut matched: Vec<&DecisionCfg> = routing
        .decisions
        .iter()
        .filter(|d| eval_rule(&d.rules, signals, projections).unwrap_or(false))
        .collect();
    if matched.is_empty() {
        return Ok(None);
    }
    match strategy {
        "confidence" => {
            matched.sort_by(|a, b| {
                let ca = confidence_of(a, signals);
                let cb = confidence_of(b, signals);
                cb.partial_cmp(&ca)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.priority.cmp(&a.priority))
            });
        }
        _ => matched.sort_by_key(|a| std::cmp::Reverse(a.priority)),
    }
    Ok(matched.into_iter().next())
}

fn confidence_of(d: &DecisionCfg, signals: &SignalSet) -> f32 {
    d.rules
        .conditions
        .iter()
        .filter_map(|c| {
            if c.kind == "projection" || c.kind == "projection_value" {
                Some(1.0)
            } else {
                signals.get(&c.kind, &c.name).map(|h| h.confidence)
            }
        })
        .fold(1.0_f32, |a, b| a.min(b))
}

pub fn eval_rule(
    node: &RuleNode,
    signals: &SignalSet,
    projections: &ProjectionMap,
) -> Result<bool, RouterError> {
    if let Some(inner) = &node.not {
        return Ok(!eval_rule(inner, signals, projections)?);
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
        .map(|c| eval_condition(c, signals, projections))
        .collect();
    Ok(match op.as_str() {
        "OR" => vals.iter().any(|v| *v),
        "NOT" => vals.iter().all(|v| !*v),
        _ => vals.iter().all(|v| *v),
    })
}

fn eval_condition(
    c: &aria_router_config::Condition,
    signals: &SignalSet,
    projections: &ProjectionMap,
) -> bool {
    if c.kind == "projection" || c.kind == "projection_value" {
        let Some(v) = projections.values.get(&c.name) else {
            return false;
        };
        match &c.equals {
            Some(want) => match v {
                Value::String(s) => s == want,
                Value::Number(n) => n.to_string() == *want,
                Value::Bool(b) => (if *b { "true" } else { "false" }) == want.as_str(),
                Value::Null => want == "null",
                _ => false,
            },
            None => !v.is_null(),
        }
    } else {
        signals.matched(&c.kind, &c.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_config::{Condition, DecisionCfg, ModelRef, ProjectionCfg, Routing};
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
            conditions: vec![Condition {
                kind: "keyword".into(),
                name: "a".into(),
                equals: None,
            }],
            not: None,
        };
        assert!(eval_rule(&node, &set, &ProjectionMap::default()).unwrap());
    }

    #[test]
    fn projection_equals_selects_decision() {
        let recipe = Recipe {
            name: "mom".into(),
            router: aria_router_core::RouterKind::Semantic,
            routing: Some(Routing {
                strategy: "priority".into(),
                model_cards: None,
                signals: Default::default(),
                projections: vec![ProjectionCfg {
                    name: "explain_band".into(),
                    kind: "mapping".into(),
                    extra: [
                        ("from".into(), serde_json::json!("explain_score")),
                        (
                            "bands".into(),
                            serde_json::json!([
                                {"name": "high", "min": 0.8, "max": 1.0},
                                {"name": "default", "min": 0.0, "max": 0.8}
                            ]),
                        ),
                    ]
                    .into(),
                }],
                decisions: vec![
                    DecisionCfg {
                        name: "band_high".into(),
                        description: None,
                        priority: 100,
                        rules: RuleNode {
                            operator: Some("AND".into()),
                            conditions: vec![Condition {
                                kind: "projection".into(),
                                name: "explain_band".into(),
                                equals: Some("high".into()),
                            }],
                            not: None,
                        },
                        model_refs: vec![ModelRef {
                            model: "large".into(),
                        }],
                        algorithm: Some("static".into()),
                        plugins: vec![],
                        locality: None,
                    },
                    DecisionCfg {
                        name: "fallback".into(),
                        description: None,
                        priority: 1,
                        rules: RuleNode {
                            operator: Some("AND".into()),
                            conditions: vec![],
                            not: None,
                        },
                        model_refs: vec![ModelRef {
                            model: "small".into(),
                        }],
                        algorithm: Some("static".into()),
                        plugins: vec![],
                        locality: None,
                    },
                ],
                algorithms: serde_json::Value::Null,
                plugins: serde_json::Value::Null,
            }),
            agent: None,
        };
        let mut signals = SignalSet::default();
        signals.hits.push(SignalHit {
            kind: "keyword".into(),
            name: "needs_explain".into(),
            matched: true,
            confidence: 1.0,
        });
        // Manually build projection map as if score mapping ran.
        let mut proj = ProjectionMap::default();
        proj.values
            .insert("explain_band".into(), Value::String("high".into()));
        let d = select_decision(&recipe, &signals, &proj, "priority")
            .unwrap()
            .unwrap();
        assert_eq!(d.name, "band_high");

        proj.values
            .insert("explain_band".into(), Value::String("default".into()));
        let d = select_decision(&recipe, &signals, &proj, "priority")
            .unwrap()
            .unwrap();
        assert_eq!(d.name, "fallback");
    }
}
