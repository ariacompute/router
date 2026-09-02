//! Selection algorithms (static / latency-aware / multi-factor).

use aria_router_config::{DecisionCfg, RouterDocument};
use aria_router_core::{RouterError, ModelCard};

#[derive(Debug, Clone, Default)]
pub struct RuntimeStats {
    pub latency_ms: std::collections::HashMap<String, f32>,
    pub load: std::collections::HashMap<String, f32>,
    pub cost: std::collections::HashMap<String, f32>,
}

pub fn select(
    _doc: &RouterDocument,
    decision: &DecisionCfg,
    eligible: &[ModelCard],
    stats: &RuntimeStats,
) -> Result<String, RouterError> {
    if eligible.is_empty() {
        return Err(RouterError::FailClosed("no eligible models".into()));
    }
    let algo = decision.algorithm.as_deref().unwrap_or("static");
    if RouterDocument::unimplemented_algorithm(algo) {
        return Err(RouterError::Unsupported(format!("algorithm {algo} not implemented")));
    }
    let names: Vec<String> = if decision.model_refs.is_empty() {
        eligible.iter().map(|m| m.name.clone()).collect()
    } else {
        decision
            .model_refs
            .iter()
            .map(|r| r.model.clone())
            .filter(|n| eligible.iter().any(|e| e.name == *n))
            .collect()
    };
    if names.is_empty() {
        return Err(RouterError::FailClosed(
            "decision modelRefs not in eligible pool".into(),
        ));
    }
    match algo {
        "static" => Ok(names[0].clone()),
        "latency-aware" | "latency_aware" => {
            let best = names
                .iter()
                .min_by(|a, b| {
                    let la = stats.latency_ms.get(*a).copied().unwrap_or(1000.0);
                    let lb = stats.latency_ms.get(*b).copied().unwrap_or(1000.0);
                    la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
                .unwrap();
            Ok(best)
        }
        "multi-factor" | "multi_factor" => {
            let best = names
                .iter()
                .min_by(|a, b| {
                    let sa = score(a, stats);
                    let sb = score(b, stats);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
                .unwrap();
            Ok(best)
        }
        other => Err(RouterError::Unsupported(format!("algorithm {other}"))),
    }
}

fn score(name: &str, stats: &RuntimeStats) -> f32 {
    let lat = stats.latency_ms.get(name).copied().unwrap_or(100.0);
    let load = stats.load.get(name).copied().unwrap_or(0.0);
    let cost = stats.cost.get(name).copied().unwrap_or(1.0);
    lat * 0.5 + load * 20.0 + cost * 10.0
}

pub fn hard_filter(
    doc: &RouterDocument,
    names: &[String],
    require_locality: Option<&str>,
    require_modality: Option<&str>,
) -> Vec<ModelCard> {
    names
        .iter()
        .filter_map(|n| doc.provider(n))
        .filter(|p| {
            if let Some(loc) = require_locality {
                if p.locality != loc {
                    return false;
                }
            }
            if let Some(mod_) = require_modality {
                if p.modality != mod_ && p.modality != "any" {
                    return false;
                }
            }
            true
        })
        .map(|p| ModelCard {
            name: p.name.clone(),
            locality: p.locality.clone(),
            modality: p.modality.clone(),
            capabilities: p.capabilities.clone(),
            provider_model_id: p.provider_model_id.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards() -> Vec<ModelCard> {
        vec![
            ModelCard {
                name: "a".into(),
                locality: "local".into(),
                modality: "text".into(),
                capabilities: vec!["chat".into()],
                provider_model_id: "a".into(),
            },
            ModelCard {
                name: "b".into(),
                locality: "local".into(),
                modality: "text".into(),
                capabilities: vec!["chat".into()],
                provider_model_id: "b".into(),
            },
        ]
    }

    fn decision(algo: &str) -> DecisionCfg {
        DecisionCfg {
            name: "d".into(),
            description: None,
            priority: 1,
            rules: Default::default(),
            model_refs: vec![
                aria_router_config::ModelRef { model: "a".into() },
                aria_router_config::ModelRef { model: "b".into() },
            ],
            algorithm: Some(algo.into()),
            plugins: vec![],
            locality: None,
        }
    }

    fn doc() -> RouterDocument {
        RouterDocument::from_yaml_str(
            r#"
version: v0.3
providers:
  models:
    - name: a
      locality: local
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
    - name: b
      locality: local
      backend_refs: [{name: p, endpoint: 127.0.0.1:2}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: r
recipes:
  - name: r
    router: semantic
    routing:
      decisions:
        - name: d
          rules: { operator: AND, conditions: [] }
          modelRefs: [{model: a}]
"#,
        )
        .unwrap()
    }

    #[test]
    fn static_first() {
        let d = doc();
        let got = select(&d, &decision("static"), &cards(), &RuntimeStats::default()).unwrap();
        assert_eq!(got, "a");
    }

    #[test]
    fn latency_aware_picks_faster() {
        let d = doc();
        let mut stats = RuntimeStats::default();
        stats.latency_ms.insert("a".into(), 200.0);
        stats.latency_ms.insert("b".into(), 10.0);
        let got = select(&d, &decision("latency-aware"), &cards(), &stats).unwrap();
        assert_eq!(got, "b");
    }

    #[test]
    fn multi_factor_picks_cheaper() {
        let d = doc();
        let mut stats = RuntimeStats::default();
        stats.cost.insert("a".into(), 9.0);
        stats.cost.insert("b".into(), 1.0);
        let got = select(&d, &decision("multi-factor"), &cards(), &stats).unwrap();
        assert_eq!(got, "b");
    }

    #[test]
    fn unimplemented_algorithm() {
        let d = doc();
        let err = select(&d, &decision("knn"), &cards(), &RuntimeStats::default()).unwrap_err();
        assert!(matches!(err, RouterError::Unsupported(_)));
    }
}
