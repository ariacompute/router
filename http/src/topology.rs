//! Graph derived from the in-memory YAML document.

use aria_router_config::{RouterDocument, Signals};
use aria_router_core::RouterKind;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub fn topology_graph(doc: &RouterDocument) -> Value {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for m in &doc.providers.models {
        add_node(
            &mut nodes,
            &mut seen,
            format!("model:{}", m.name),
            "model",
            &m.name,
            &[("locality", json!(m.locality))],
        );
    }

    for recipe in &doc.recipes {
        let rid = format!("recipe:{}", recipe.name);
        add_node(
            &mut nodes,
            &mut seen,
            rid.clone(),
            "recipe",
            &recipe.name,
            &[("router", json!(recipe.router.as_str()))],
        );
        match recipe.router {
            RouterKind::Semantic => {
                if let Some(routing) = &recipe.routing {
                    for (sid, label, kind) in signal_nodes(&routing.signals) {
                        add_node(
                            &mut nodes,
                            &mut seen,
                            sid.clone(),
                            "signal",
                            &label,
                            &[("signal", json!(kind))],
                        );
                        edges.push(json!({"from": rid, "to": sid}));
                    }
                    for d in &routing.decisions {
                        let did = format!("decision:{}", d.name);
                        add_node(&mut nodes, &mut seen, did.clone(), "decision", &d.name, &[]);
                        edges.push(json!({"from": rid, "to": did}));
                        let algo = d.algorithm.as_deref().unwrap_or("static");
                        let aid = format!("algorithm:{algo}");
                        add_node(&mut nodes, &mut seen, aid.clone(), "algorithm", algo, &[]);
                        edges.push(json!({"from": did, "to": aid}));
                        for p in &d.plugins {
                            let pid = format!("plugin:{}", p.name);
                            add_node(&mut nodes, &mut seen, pid.clone(), "plugin", &p.name, &[]);
                            edges.push(json!({"from": did, "to": pid}));
                        }
                        for m in &d.model_refs {
                            edges.push(json!({"from": aid, "to": format!("model:{}", m.model)}));
                        }
                    }
                }
            }
            RouterKind::Agent => {
                if let Some(agent) = &recipe.agent {
                    let xid = "builtin".to_string();
                    let nid = format!("builtin:{xid}");
                    add_node(
                        &mut nodes,
                        &mut seen,
                        nid.clone(),
                        "builtin",
                        "builtin",
                        &[
                            ("max_turns", json!(agent.max_turns.unwrap_or(3))),
                            ("timeout_ms", json!(agent.timeout_ms.unwrap_or(5000))),
                        ],
                    );
                    edges.push(json!({"from": rid, "to": nid}));
                    if let Some(fb) = &agent.fallback {
                        edges.push(json!({"from": nid, "to": format!("model:{fb}")}));
                    }
                }
            }
            RouterKind::AfmD => {
                if let Some(afm) = &recipe.afm_d {
                    let nid = "afm-d:systemone".to_string();
                    add_node(
                        &mut nodes,
                        &mut seen,
                        nid.clone(),
                        "afm-d",
                        "afm-d",
                        &[
                            ("timeout_ms", json!(afm.timeout_ms.unwrap_or(5000))),
                            ("endpoint", json!(afm.endpoint.as_deref().unwrap_or(""))),
                        ],
                    );
                    edges.push(json!({"from": rid, "to": nid}));
                    if let Some(fb) = &afm.fallback {
                        edges.push(json!({"from": nid, "to": format!("model:{fb}")}));
                    }
                }
            }
        }
    }

    for ep in &doc.entrypoints {
        let label = ep.model_names.join(", ");
        let eid = format!("entrypoint:{label}");
        add_node(
            &mut nodes,
            &mut seen,
            eid.clone(),
            "entrypoint",
            &label,
            &[("router", json!(ep.router.as_str()))],
        );
        edges.push(json!({"from": eid, "to": format!("recipe:{}", ep.recipe)}));
    }

    json!({"nodes": nodes, "edges": edges})
}

fn add_node(
    nodes: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    id: String,
    kind: &str,
    label: &str,
    extra: &[(&str, Value)],
) {
    if !seen.insert(id.clone()) {
        return;
    }
    let mut map = Map::new();
    map.insert("id".into(), json!(id));
    map.insert("kind".into(), json!(kind));
    map.insert("label".into(), json!(label));
    for (k, v) in extra {
        map.insert((*k).into(), v.clone());
    }
    nodes.push(Value::Object(map));
}

fn signal_nodes(signals: &Signals) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for kw in &signals.keywords {
        out.push((
            format!("signal:keyword:{}", kw.name),
            kw.name.clone(),
            "keyword".into(),
        ));
    }
    for s in &signals.language {
        out.push((
            format!("signal:language:{}", s.name),
            s.name.clone(),
            "language".into(),
        ));
    }
    for s in &signals.context {
        out.push((
            format!("signal:context:{}", s.name),
            s.name.clone(),
            "context".into(),
        ));
    }
    for s in &signals.authz {
        out.push((
            format!("signal:authz:{}", s.name),
            s.name.clone(),
            "authz".into(),
        ));
    }
    for s in &signals.conversation {
        out.push((
            format!("signal:conversation:{}", s.name),
            s.name.clone(),
            "conversation".into(),
        ));
    }
    for s in &signals.metadata {
        out.push((
            format!("signal:metadata:{}", s.name),
            s.name.clone(),
            "metadata".into(),
        ));
    }
    for s in &signals.event {
        out.push((
            format!("signal:event:{}", s.name),
            s.name.clone(),
            "event".into(),
        ));
    }
    for s in &signals.structure {
        out.push((
            format!("signal:structure:{}", s.name),
            s.name.clone(),
            "structure".into(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
version: v0.3
providers:
  models:
    - name: small
      locality: local
      backend_refs: [{name: p, endpoint: 127.0.0.1:1}]
    - name: large
      locality: cloud
      backend_refs: [{name: p, endpoint: 127.0.0.1:2}]
entrypoints:
  - model_names: [auto]
    router: semantic
    recipe: sem
  - model_names: [agent-mode]
    router: agent
    recipe: ag
  - model_names: [afm]
    router: afm-d
    recipe: af
recipes:
  - name: sem
    router: semantic
    routing:
      signals:
        keywords: [{name: kw, keywords: [explain]}]
        language: [{name: lang, code: en}]
        context: [{name: ctx}]
        authz: [{name: az, role: admin}]
        conversation: [{name: conv}]
        metadata: [{name: md, key: k, value: v}]
        event: [{name: ev}]
        structure: [{name: st}]
      decisions:
        - name: d
          rules: {operator: AND, conditions: []}
          algorithm: latency-aware
          plugins: [{name: header-mutation}]
          modelRefs: [{model: small}]
        - name: d2
          rules: {operator: AND, conditions: []}
          modelRefs: [{model: large}]
        - name: d3
          rules: {operator: AND, conditions: []}
          algorithm: static
          modelRefs: [{model: small}]
  - name: ag
    router: agent
    agent:
      max_turns: 2
      timeout_ms: 100
      fallback: small
  - name: af
    router: afm-d
    afm-d:
      timeout_ms: 200
      endpoint: "http://127.0.0.1:8010"
      fallback: large
"#;

    fn ids(g: &Value) -> Vec<String> {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn has_edge(g: &Value, from: &str, to: &str) -> bool {
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["from"] == from && e["to"] == to)
    }

    #[test]
    fn graphs_every_router_kind_and_signal_family() {
        let doc = RouterDocument::from_yaml_str(YAML).unwrap();
        let g = topology_graph(&doc);
        let ids = ids(&g);
        for want in [
            "model:small",
            "model:large",
            "recipe:sem",
            "recipe:ag",
            "recipe:af",
            "signal:keyword:kw",
            "signal:language:lang",
            "signal:context:ctx",
            "signal:authz:az",
            "signal:conversation:conv",
            "signal:metadata:md",
            "signal:event:ev",
            "signal:structure:st",
            "decision:d",
            "decision:d2",
            "algorithm:latency-aware",
            "algorithm:static",
            "plugin:header-mutation",
            "builtin:builtin",
            "afm-d:systemone",
            "entrypoint:auto",
            "entrypoint:agent-mode",
            "entrypoint:afm",
        ] {
            assert!(ids.contains(&want.to_string()), "missing node {want}");
        }
        assert!(has_edge(&g, "recipe:sem", "signal:keyword:kw"));
        assert!(has_edge(&g, "decision:d", "algorithm:latency-aware"));
        assert!(has_edge(&g, "decision:d", "plugin:header-mutation"));
        assert!(has_edge(&g, "algorithm:latency-aware", "model:small"));
        assert!(has_edge(&g, "builtin:builtin", "model:small"));
        assert!(has_edge(&g, "afm-d:systemone", "model:large"));
        assert!(has_edge(&g, "entrypoint:auto", "recipe:sem"));
        assert!(has_edge(&g, "recipe:af", "afm-d:systemone"));
    }

    #[test]
    fn shared_nodes_are_emitted_once() {
        // `algorithm:static` is produced by two decisions; the node is deduped
        // while each decision→algorithm edge is still recorded.
        let doc = RouterDocument::from_yaml_str(YAML).unwrap();
        let g = topology_graph(&doc);
        let nodes = g["nodes"].as_array().unwrap();
        let statics = nodes
            .iter()
            .filter(|n| n["id"] == "algorithm:static")
            .count();
        assert_eq!(statics, 1);
        let edges = g["edges"].as_array().unwrap();
        let static_edges = edges
            .iter()
            .filter(|e| e["to"] == "algorithm:static")
            .count();
        assert_eq!(static_edges, 2);
    }

    #[test]
    fn semantic_recipe_without_routing_emits_no_signal_nodes() {
        let mut doc = RouterDocument::from_yaml_str(YAML).unwrap();
        for r in &mut doc.recipes {
            if r.name == "sem" {
                r.routing = None;
            }
        }
        let g = topology_graph(&doc);
        let nodes = ids(&g);
        assert!(nodes.contains(&"recipe:sem".to_string()));
        assert!(!nodes.iter().any(|n| n.starts_with("signal:")));
        assert!(!nodes.iter().any(|n| n.starts_with("decision:")));
    }

    #[test]
    fn agent_and_afm_d_without_options_emit_no_extra_nodes() {
        // Validation rejects agent/afm-d recipes without their block, so a
        // missing block can only be reached by clearing it after parsing.
        let mut doc = RouterDocument::from_yaml_str(YAML).unwrap();
        for r in &mut doc.recipes {
            r.agent = None;
            r.afm_d = None;
        }
        let g = topology_graph(&doc);
        let nodes = ids(&g);
        assert!(nodes.contains(&"recipe:ag".to_string()));
        assert!(nodes.contains(&"recipe:af".to_string()));
        assert!(!nodes.iter().any(|n| n.starts_with("builtin:")));
        assert!(!nodes.iter().any(|n| n.starts_with("afm-d:")));
    }
}
