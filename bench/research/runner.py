"""Research bench runner: DRACO-shaped JSONL → systems × domains report."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..http_client import ChatFn, EndpointConfig, chat_completion, probe_models
from ..router_targets import RouterSpec
from .scorer import mean_axes, score_research_item


def load_research_corpus(path: str | Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    text = Path(path).read_text(encoding="utf-8")
    for i, line in enumerate(text.splitlines()):
        line = line.strip()
        if not line:
            continue
        obj = json.loads(line)
        if not isinstance(obj, dict):
            raise ValueError(f"line {i+1}: expected object")
        for req in ("problem", "answer"):
            if req not in obj:
                raise ValueError(f"line {i+1}: missing {req}")
        obj.setdefault("id", f"r{i}")
        obj.setdefault("domain", "general")
        if "expected_hits" not in obj and isinstance(obj["answer"], dict):
            if "expected_hits" in obj["answer"]:
                obj["expected_hits"] = obj["answer"].pop("expected_hits")
        rows.append(obj)
    return rows


def _normalize_routers(
    routers: Sequence[RouterSpec] | None,
    router: EndpointConfig | None,
    entrypoint: str,
) -> list[RouterSpec]:
    if routers:
        return list(routers)
    if router is not None:
        return [
            RouterSpec(
                name="aria_router",
                endpoint=router,
                entrypoint=entrypoint,
                pick_headers=["x-aria-router-model"],
            )
        ]
    return []


def run_research(
    *,
    corpus: list[dict[str, Any]],
    pool: Mapping[str, EndpointConfig],
    model_ids: Mapping[str, str],
    quality: str = "label",
    routers: Sequence[RouterSpec] | None = None,
    router: EndpointConfig | None = None,
    entrypoint: str = "ariacompute/semantic-auto",
    max_tokens: int = 512,
    skip_probe: bool = False,
    judge_url: str | None = None,
    judge_model: str = "judge",
    judge_api_key: str = "",
    chat_fn: ChatFn | None = None,
) -> dict[str, Any]:
    if quality not in ("label", "overlap", "judge"):
        raise ValueError(f"quality must be label|overlap|judge, got {quality!r}")
    router_list = _normalize_routers(routers, router, entrypoint)
    if not pool and not router_list:
        raise ValueError("need --pool and/or --router")

    fn = chat_fn or chat_completion
    notes: list[str] = []
    probes: dict[str, Any] = {}

    alias_to_model = {a: model_ids.get(a, a) for a in pool.keys()}

    if not skip_probe:
        for alias, cfg in pool.items():
            ok, detail = probe_models(cfg)
            probes[alias] = {"ok": ok, "detail": detail}
        for rs in router_list:
            ok, detail = probe_models(rs.endpoint)
            probes[f"router:{rs.name}"] = {"ok": ok, "detail": detail}

    # (system_name, model_to_request, endpoint, pick_headers|None)
    systems: list[tuple[str, str, EndpointConfig, list[str] | None]] = []
    for alias, cfg in pool.items():
        mid = alias_to_model[alias]
        systems.append((f"always_{mid}", mid, cfg, None))
    for rs in router_list:
        systems.append((rs.name, rs.entrypoint, rs.endpoint, rs.pick_headers))
    if not router_list:
        notes.append("no --router; live router systems omitted")

    results: list[dict[str, Any]] = []
    by_system: dict[str, list[dict[str, Any]]] = {name: [] for name, _, _, _ in systems}
    by_system_domain: dict[str, dict[str, list[dict[str, Any]]]] = {
        name: {} for name, _, _, _ in systems
    }

    for item in corpus:
        tid = str(item["id"])
        domain = str(item.get("domain") or "general")
        problem = item["problem"]
        answer = item["answer"]
        expected_hits = item.get("expected_hits")
        if isinstance(expected_hits, str):
            expected_hits = [expected_hits]

        for sys_name, model, cfg, pick_headers in systems:
            chat = fn(
                cfg, model=model, prompt=problem, max_tokens=max_tokens, temperature=0.0
            )
            row: dict[str, Any] = {
                "id": tid,
                "domain": domain,
                "system": sys_name,
                "request_model": model,
            }
            if chat.status != "ok":
                row.update(
                    {
                        "status": "error",
                        "error": chat.error,
                        "score": 0.0,
                        "axes": {},
                    }
                )
                results.append(row)
                continue

            if pick_headers is not None:
                routed = chat.picked_model(pick_headers)
            else:
                routed = model
            row["routed_model"] = routed
            scored = score_research_item(
                problem=problem,
                completion=chat.content,
                answer=answer,
                quality=quality,
                expected_hits=expected_hits,
                judge_url=judge_url,
                judge_model=judge_model,
                judge_api_key=judge_api_key,
                timeout_s=cfg.timeout_s,
                chat_fn=fn,
            )
            row["status"] = scored.get("status", "ok")
            row["score"] = scored.get("score", 0.0)
            row["axes"] = scored.get("axes") or {}
            if scored.get("reason"):
                row["reason"] = scored["reason"]
            if scored.get("sections"):
                row["sections"] = scored["sections"]
            results.append(row)
            if row["status"] == "ok":
                by_system[sys_name].append(row)
                by_system_domain[sys_name].setdefault(domain, []).append(row)

    system_summaries: list[dict[str, Any]] = []
    always_scores: dict[str, float] = {}
    for sys_name, _, _, _ in systems:
        rows = by_system.get(sys_name) or []
        mean_score = sum(float(r["score"]) for r in rows) / len(rows) if rows else 0.0
        summary = {
            "system": sys_name,
            "mean_score": mean_score,
            "axes": mean_axes(rows),
            "n": len(rows),
            "by_domain": {
                d: {
                    "mean_score": (
                        sum(float(r["score"]) for r in rs) / len(rs) if rs else 0.0
                    ),
                    "axes": mean_axes(rs),
                    "n": len(rs),
                }
                for d, rs in (by_system_domain.get(sys_name) or {}).items()
            },
        }
        system_summaries.append(summary)
        if sys_name.startswith("always_"):
            always_scores[sys_name] = mean_score

    best_always = max(always_scores.values()) if always_scores else None
    for s in system_summaries:
        if best_always is not None:
            s["delta_vs_best_always"] = float(s["mean_score"]) - best_always
        else:
            s["delta_vs_best_always"] = None

    skipped = sum(1 for r in results if r.get("status") == "skipped")
    errors = sum(1 for r in results if r.get("status") == "error")

    return {
        "mode": "router_research",
        "ci_fail": False,
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "quality": quality,
            "routers": [
                {
                    "name": rs.name,
                    "base_url": rs.endpoint.base_url,
                    "entrypoint": rs.entrypoint,
                    "pick_headers": rs.pick_headers,
                }
                for rs in router_list
            ],
            "entrypoint": router_list[0].entrypoint if len(router_list) == 1 else None,
            "tasks": len(corpus),
            "systems": [n for n, _, _, _ in systems],
        },
        "summary": {
            "tasks": len(corpus),
            "results_ok": sum(1 for r in results if r.get("status") == "ok"),
            "results_skipped": skipped,
            "results_error": errors,
            "systems": len(systems),
        },
        "probes": probes,
        "systems": system_summaries,
        "results": results,
        "notes": notes,
    }
