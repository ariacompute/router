//! Graph derived from the in-memory YAML document.

use ariarouter_config::{RouterDocument, Signals};
use ariarouter_core::RouterKind;
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
                    let xid = format!("extension:{}", agent.extension);
                    add_node(
                        &mut nodes,
                        &mut seen,
                        xid.clone(),
                        "extension",
                        &agent.extension,
                        &[],
                    );
                    edges.push(json!({"from": rid, "to": xid}));
                    if let Some(fb) = &agent.fallback {
                        edges.push(json!({"from": xid, "to": format!("model:{fb}")}));
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
