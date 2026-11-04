//! Boolean decisions + projections.

use aria_router_config::{CompetenceCfg, DecisionCfg, ProjectionCfg, Recipe, RuleNode};
use aria_router_core::{ModelCard, RouterError};
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
                let winner = inputs
                    .into_iter()
                    .find(|name| signals.hits.iter().any(|h| h.name == *name && h.matched));
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

/// Competence-aware downgrade.
///
/// Given the selected model and the post-hard-constraint `eligible` pool, returns
/// `Some(cheaper_model)` when all of:
/// - the feature is `enabled`,
/// - `competence` is known (caller passes `None` when unknown → no downgrade),
/// - `simple_prob >= cfg.simple_threshold`,
/// - `competence >= cfg.floor`,
///   and there exists an eligible model whose tier rank is strictly cheaper than the
///   selected model's. Otherwise returns `None` (keep the matched model). Fail-closed
///   behavior is preserved because this only ever swaps *within* the already-eligible
///   pool.
pub fn apply_competence(
    selected: &str,
    eligible: &[ModelCard],
    simple_prob: f32,
    competence: Option<f32>,
    cfg: &CompetenceCfg,
) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    let competence = competence?; // unknown → forbid downgrade
    if simple_prob < cfg.simple_threshold {
        return None;
    }
    if competence < cfg.floor {
        return None;
    }
    let selected_card = eligible.iter().find(|m| m.name == selected)?;
    let selected_rank = tier_rank(Some(selected_card));
    let mut best: Option<(u8, &ModelCard)> = None;
    for m in eligible {
        if m.name == selected {
            continue;
        }
        let r = tier_rank(Some(m));
        if r < selected_rank {
            match best {
                None => best = Some((r, m)),
                Some((br, _)) if r < br => best = Some((r, m)),
                _ => {}
            }
        }
    }
    best.map(|(_, m)| m.name.clone())
}

/// Lower number = cheaper tier. Unknown tiers are treated as `mid` (rank 1) so we
/// never downgrade *to* an unknown tier nor assume an unknown selected model is cheap.
fn tier_rank(m: Option<&ModelCard>) -> u8 {
    match m.and_then(|m| m.tier.as_deref()) {
        Some("small") => 0,
        Some("mid") | Some("medium") => 1,
        Some("large") => 2,
        _ => 1,
    }
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
    fn or_not_and_missing_projection() {
        let mut set = SignalSet::default();
        set.hits.push(SignalHit {
            kind: "keyword".into(),
            name: "a".into(),
            matched: true,
            confidence: 1.0,
        });
        let or_node = RuleNode {
            operator: Some("OR".into()),
            conditions: vec![
                Condition {
                    kind: "keyword".into(),
                    name: "missing".into(),
                    equals: None,
                },
                Condition {
                    kind: "keyword".into(),
                    name: "a".into(),
                    equals: None,
                },
            ],
            not: None,
        };
        assert!(eval_rule(&or_node, &set, &ProjectionMap::default()).unwrap());

        let not_node = RuleNode {
            operator: Some("AND".into()),
            conditions: vec![],
            not: Some(Box::new(RuleNode {
                operator: Some("AND".into()),
                conditions: vec![Condition {
                    kind: "keyword".into(),
                    name: "a".into(),
                    equals: None,
                }],
                not: None,
            })),
        };
        assert!(!eval_rule(&not_node, &set, &ProjectionMap::default()).unwrap());

        let proj_cond = RuleNode {
            operator: Some("AND".into()),
            conditions: vec![Condition {
                kind: "projection".into(),
                name: "absent".into(),
                equals: Some("x".into()),
            }],
            not: None,
        };
        assert!(!eval_rule(&proj_cond, &set, &ProjectionMap::default()).unwrap());

        let mut proj = ProjectionMap::default();
        proj.values.insert("n".into(), Value::Null);
        let null_eq = RuleNode {
            operator: Some("AND".into()),
            conditions: vec![Condition {
                kind: "projection".into(),
                name: "n".into(),
                equals: Some("null".into()),
            }],
            not: None,
        };
        assert!(eval_rule(&null_eq, &set, &proj).unwrap());
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
                        emits: vec![],
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
                        emits: vec![],
                    },
                ],
                algorithms: serde_json::Value::Null,
                plugins: serde_json::Value::Null,
                prefix_reuse: None,
                competence: None,
                decision_ext: None,
            }),
            agent: None,
            afm_d: None,
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

    fn card(name: &str, tier: &str) -> ModelCard {
        ModelCard {
            name: name.into(),
            locality: "local".into(),
            modality: "text".into(),
            capabilities: vec!["chat".into()],
            provider_model_id: String::new(),
            tier: Some(tier.into()),
        }
    }

    #[test]
    fn competence_downgrade_prefers_cheaper_tier() {
        let eligible = vec![
            card("ariacompute/ariamodel-large", "large"),
            card("ariacompute/ariamodel-mid", "mid"),
            card("ariacompute/ariamodel-small", "small"),
        ];
        let cfg = CompetenceCfg {
            enabled: true,
            floor: 0.6,
            simple_threshold: 0.7,
        };
        // Simple + competent → downgrade large → small.
        let down = apply_competence(
            "ariacompute/ariamodel-large",
            &eligible,
            0.9,
            Some(0.95),
            &cfg,
        )
        .unwrap();
        assert_eq!(down, "ariacompute/ariamodel-small");

        // Simple but low competence → no downgrade.
        assert!(apply_competence(
            "ariacompute/ariamodel-large",
            &eligible,
            0.9,
            Some(0.4),
            &cfg
        )
        .is_none());

        // Unknown competence → no downgrade (fail-closed style).
        assert!(
            apply_competence("ariacompute/ariamodel-large", &eligible, 0.9, None, &cfg).is_none()
        );

        // Not simple enough → no downgrade.
        assert!(apply_competence(
            "ariacompute/ariamodel-large",
            &eligible,
            0.3,
            Some(0.95),
            &cfg
        )
        .is_none());

        // Already cheapest → no downgrade.
        assert!(apply_competence(
            "ariacompute/ariamodel-small",
            &eligible,
            0.9,
            Some(0.95),
            &cfg
        )
        .is_none());

        // Disabled → no downgrade.
        let off = CompetenceCfg {
            enabled: false,
            ..cfg
        };
        assert!(apply_competence(
            "ariacompute/ariamodel-large",
            &eligible,
            0.9,
            Some(0.95),
            &off
        )
        .is_none());
    }

    fn hit(kind: &str, name: &str, matched: bool, confidence: f32) -> SignalHit {
        SignalHit {
            kind: kind.into(),
            name: name.into(),
            matched,
            confidence,
        }
    }

    fn projection(name: &str, kind: &str, extra: serde_json::Value) -> ProjectionCfg {
        let map: HashMap<String, serde_json::Value> =
            serde_json::from_value(extra).unwrap_or_default();
        ProjectionCfg {
            name: name.into(),
            kind: kind.into(),
            extra: map,
        }
    }

    #[test]
    fn project_partition_picks_first_matched_signal() {
        let set = SignalSet {
            hits: vec![
                hit("keyword", "slow", false, 1.0),
                hit("keyword", "fast", true, 0.4),
            ],
        };
        let cfgs = vec![projection(
            "speed",
            "partition",
            serde_json::json!({"signals": ["slow", "fast", "missing"]}),
        )];
        let pm = project(&cfgs, &set).unwrap();
        assert_eq!(pm.values["speed"], Value::String("fast".into()));

        // No `signals` array → empty input list → null (no partition winner).
        let cfgs = vec![projection("speed", "partition", serde_json::json!({}))];
        let pm = project(&cfgs, &set).unwrap();
        assert_eq!(pm.values["speed"], Value::Null);
    }

    #[test]
    fn project_score_and_mapping_bands() {
        let set = SignalSet {
            hits: vec![hit("keyword", "kw", true, 0.8)],
        };
        let cfgs = vec![
            projection(
                "score",
                "score",
                serde_json::json!({"weights": {"kw": 0.5, "absent": 9.0}}),
            ),
            projection(
                "band",
                "mapping",
                serde_json::json!({
                    "from": "score",
                    "bands": [
                        {"name": "low", "min": 0.0, "max": 0.1},
                        {"name": "high", "min": 0.1, "max": 1.0}
                    ]
                }),
            ),
        ];
        let pm = project(&cfgs, &set).unwrap();
        assert!((pm.values["score"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        assert_eq!(pm.values["band"], Value::String("high".into()));
    }

    #[test]
    fn project_mapping_defaults_when_unbanded() {
        // No bands at all → "default".
        let cfgs = vec![projection(
            "band",
            "mapping",
            serde_json::json!({"from": "missing"}),
        )];
        let pm = project(&cfgs, &SignalSet::default()).unwrap();
        assert_eq!(pm.values["band"], Value::String("default".into()));

        // Band without name/min/max defaults to an open interval named "band".
        let cfgs = vec![projection(
            "band",
            "mapping",
            serde_json::json!({"from": "missing", "bands": [{}]}),
        )];
        let pm = project(&cfgs, &SignalSet::default()).unwrap();
        assert_eq!(pm.values["band"], Value::String("band".into()));
    }

    #[test]
    fn project_score_without_weights_is_zero() {
        let cfgs = vec![projection("score", "score", serde_json::json!({}))];
        let pm = project(&cfgs, &SignalSet::default()).unwrap();
        assert_eq!(pm.values["score"], serde_json::json!(0.0));

        // Weights that are not an object are ignored too.
        let cfgs = vec![projection(
            "score",
            "score",
            serde_json::json!({"weights": 3}),
        )];
        let pm = project(&cfgs, &SignalSet::default()).unwrap();
        assert_eq!(pm.values["score"], serde_json::json!(0.0));
    }

    #[test]
    fn project_unknown_kind_is_unsupported() {
        let cfgs = vec![projection("x", "cluster", serde_json::json!({}))];
        let err = project(&cfgs, &SignalSet::default()).unwrap_err();
        assert!(matches!(err, RouterError::Unsupported(_)));
    }

    fn recipe_with(strategy: &str, decisions: Vec<DecisionCfg>, routing: bool) -> Recipe {
        Recipe {
            name: "r".into(),
            router: aria_router_core::RouterKind::Semantic,
            routing: routing.then(|| Routing {
                strategy: strategy.into(),
                model_cards: None,
                signals: Default::default(),
                projections: vec![],
                decisions,
                algorithms: serde_json::Value::Null,
                plugins: serde_json::Value::Null,
                prefix_reuse: None,
                competence: None,
                decision_ext: None,
            }),
            agent: None,
            afm_d: None,
        }
    }

    fn decision_cfg(name: &str, priority: i32, conditions: Vec<Condition>) -> DecisionCfg {
        DecisionCfg {
            name: name.into(),
            description: None,
            priority,
            rules: RuleNode {
                operator: Some("AND".into()),
                conditions,
                not: None,
            },
            model_refs: vec![ModelRef { model: "m".into() }],
            algorithm: Some("static".into()),
            plugins: vec![],
            locality: None,
            emits: vec![],
        }
    }

    #[test]
    fn select_decision_without_routing_or_match_returns_none() {
        let set = SignalSet::default();
        let no_routing = recipe_with("priority", vec![], false);
        assert!(
            select_decision(&no_routing, &set, &ProjectionMap::default(), "priority")
                .unwrap()
                .is_none()
        );

        let no_match = recipe_with(
            "priority",
            vec![decision_cfg(
                "d",
                1,
                vec![Condition {
                    kind: "keyword".into(),
                    name: "absent".into(),
                    equals: None,
                }],
            )],
            true,
        );
        assert!(
            select_decision(&no_match, &set, &ProjectionMap::default(), "priority")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn confidence_strategy_prefers_higher_signal_confidence() {
        let set = SignalSet {
            hits: vec![
                hit("keyword", "weak", true, 0.2),
                hit("keyword", "strong", true, 0.9),
            ],
        };
        let recipe = recipe_with(
            "confidence",
            vec![
                decision_cfg(
                    "weak_but_high_priority",
                    100,
                    vec![Condition {
                        kind: "keyword".into(),
                        name: "weak".into(),
                        equals: None,
                    }],
                ),
                decision_cfg(
                    "strong",
                    1,
                    vec![Condition {
                        kind: "keyword".into(),
                        name: "strong".into(),
                        equals: None,
                    }],
                ),
            ],
            true,
        );
        let chosen = select_decision(&recipe, &set, &ProjectionMap::default(), "confidence")
            .unwrap()
            .unwrap();
        assert_eq!(chosen.name, "strong");
    }

    #[test]
    fn confidence_strategy_breaks_ties_on_priority() {
        let set = SignalSet {
            hits: vec![
                hit("keyword", "a", true, 0.5),
                hit("keyword", "b", true, 0.5),
            ],
        };
        let recipe = recipe_with(
            "confidence",
            vec![
                decision_cfg(
                    "low",
                    1,
                    vec![Condition {
                        kind: "keyword".into(),
                        name: "a".into(),
                        equals: None,
                    }],
                ),
                decision_cfg(
                    "high",
                    9,
                    vec![
                        Condition {
                            kind: "keyword".into(),
                            name: "b".into(),
                            equals: None,
                        },
                        // Projection conditions carry full confidence.
                        Condition {
                            kind: "projection".into(),
                            name: "absent_projection".into(),
                            equals: None,
                        },
                    ],
                ),
            ],
            true,
        );
        // The projection condition has no value → the decision would not match at all,
        // so the low-priority decision alone matches and wins.
        let chosen = select_decision(&recipe, &set, &ProjectionMap::default(), "confidence")
            .unwrap()
            .unwrap();
        assert_eq!(chosen.name, "low");
    }

    #[test]
    fn confidence_of_uses_projection_constant_and_signal_confidence() {
        let set = SignalSet {
            hits: vec![hit("keyword", "a", true, 0.5)],
        };
        let mut pm = ProjectionMap::default();
        pm.values.insert("band".into(), Value::String("x".into()));
        let d = decision_cfg(
            "d",
            1,
            vec![
                Condition {
                    kind: "projection".into(),
                    name: "band".into(),
                    equals: Some("x".into()),
                },
                Condition {
                    kind: "projection_value".into(),
                    name: "band".into(),
                    equals: Some("x".into()),
                },
                Condition {
                    kind: "keyword".into(),
                    name: "a".into(),
                    equals: None,
                },
            ],
        );
        assert!((confidence_of(&d, &set) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn projection_equals_handles_number_bool_and_unnamed_values() {
        let set = SignalSet::default();
        let mut pm = ProjectionMap::default();
        pm.values.insert("num".into(), serde_json::json!(3));
        pm.values.insert("flag".into(), serde_json::json!(true));
        pm.values.insert("arr".into(), serde_json::json!([1, 2]));
        pm.values.insert("text".into(), Value::String("x".into()));

        let cond = |name: &str, equals: Option<&str>| RuleNode {
            operator: Some("AND".into()),
            conditions: vec![Condition {
                kind: "projection".into(),
                name: name.into(),
                equals: equals.map(str::to_string),
            }],
            not: None,
        };
        assert!(eval_rule(&cond("num", Some("3")), &set, &pm).unwrap());
        assert!(eval_rule(&cond("flag", Some("true")), &set, &pm).unwrap());
        assert!(!eval_rule(&cond("flag", Some("false")), &set, &pm).unwrap());
        assert!(!eval_rule(&cond("arr", Some("1")), &set, &pm).unwrap());
        // No `equals` → only presence (non-null) matters.
        assert!(eval_rule(&cond("text", None), &set, &pm).unwrap());
    }

    #[test]
    fn competence_keeps_best_and_handles_unknown_tier() {
        let cfg = CompetenceCfg {
            enabled: true,
            floor: 0.6,
            simple_threshold: 0.7,
        };
        // Cheapest candidate first: the later mid-tier candidate must not replace it.
        let eligible = vec![
            card("small", "small"),
            card("mid", "mid"),
            card("large", "large"),
        ];
        let down = apply_competence("large", &eligible, 0.9, Some(0.95), &cfg).unwrap();
        assert_eq!(down, "small");

        // Unknown tier ranks like `mid`, so a small model is still a valid downgrade.
        let mut unknown = card("plain", "mid");
        unknown.tier = None;
        let eligible = vec![unknown.clone(), card("small", "small")];
        let down = apply_competence("plain", &eligible, 0.9, Some(0.95), &cfg).unwrap();
        assert_eq!(down, "small");

        // Selected model is not in the eligible pool → None (fail closed).
        assert!(apply_competence("ghost", &eligible, 0.9, Some(0.95), &cfg).is_none());
    }
}
