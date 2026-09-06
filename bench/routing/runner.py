"""Routing bench runner: corpus → matrix → policies → report."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..http_client import (
    ChatFn,
    EndpointConfig,
    chat_completion,
    estimate_tokens,
    probe_models,
)
from ..prices import cost_of, load_prices
from ..quality.judge import judge_overall
from ..quality.label import label_score
from ..quality.overlap import token_overlap
from ..router_targets import RouterSpec, resolve_pick
from .matrix import Cell, RoutingMatrix
from .policies import (
    always_policy,
    analyse,
    domain_router,
    evaluate_policy,
    knn_router,
    oracle_cost_optimal,
    oracle_quality,
    router_policy,
)


def load_routing_corpus(path: str | Path) -> list[dict[str, Any]]:
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise ValueError("routing corpus must be a JSON list")
    out = []
    for i, row in enumerate(data):
        if not isinstance(row, dict):
            raise ValueError(f"corpus[{i}] must be object")
        if "prompt" not in row:
            raise ValueError(f"corpus[{i}] missing prompt")
        item = dict(row)
        item.setdefault("id", f"q{i}")
        out.append(item)
    return out


def _score_cell_quality(
    *,
    quality_mode: str,
    model: str,
    content: str,
    expected_model: str | None,
    ref_content: str | None,
    prompt: str,
    judge_url: str | None,
    judge_model: str,
    judge_api_key: str,
    timeout_s: float,
    chat_fn: ChatFn | None,
) -> tuple[float, dict[str, Any]]:
    detail: dict[str, Any] = {"quality_mode": quality_mode}
    if quality_mode == "label":
        q = label_score(model, expected_model)
        detail["expected_model"] = expected_model
        return q, detail
    if quality_mode == "overlap":
        if ref_content is None:
            return 0.0, {**detail, "reason": "no ref completion"}
        q = token_overlap(content, ref_content)
        return q, detail
    if quality_mode == "judge":
        j = judge_overall(
            prompt=prompt,
            completion=content,
            judge_url=judge_url,
            judge_model=judge_model,
            judge_api_key=judge_api_key,
            timeout_s=timeout_s,
            chat_fn=chat_fn,
        )
        detail["judge"] = j
        if j["status"] != "ok" or j.get("score") is None:
            return 0.0, detail
        return float(j["score"]), detail
    raise ValueError(f"unknown quality mode {quality_mode!r}")


def _normalize_routers(
    routers: Sequence[RouterSpec] | None,
    router: EndpointConfig | None,
    entrypoint: str,
) -> list[RouterSpec]:
    """Accept multi-router list or legacy single ``router`` + ``entrypoint``."""
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


def run_routing(
    *,
    corpus: list[dict[str, Any]],
    pool: Mapping[str, EndpointConfig],
    model_ids: Mapping[str, str],
    quality: str = "label",
    routers: Sequence[RouterSpec] | None = None,
    router: EndpointConfig | None = None,
    entrypoint: str = "aria/semantic-auto",
    pick_map: Mapping[str, str] | None = None,
    ref_model: str | None = None,
    prices: Mapping[str, float] | None = None,
    eps: float = 0.03,
    max_tokens: int = 128,
    skip_probe: bool = False,
    judge_url: str | None = None,
    judge_model: str = "judge",
    judge_api_key: str = "",
    chat_fn: ChatFn | None = None,
    include_domain_knn: bool = True,
) -> dict[str, Any]:
    """Build matrix, evaluate policies, return report dict."""
    if quality not in ("label", "overlap", "judge"):
        raise ValueError(f"quality must be label|overlap|judge, got {quality!r}")
    if not pool:
        raise ValueError("at least one --pool is required")

    router_list = _normalize_routers(routers, router, entrypoint)
    pmap = dict(pick_map) if pick_map else {}
    price_table = dict(prices) if prices is not None else load_prices(None)
    fn = chat_fn or chat_completion
    notes: list[str] = []

    alias_to_model = {alias: model_ids.get(alias, alias) for alias in pool.keys()}
    seen: set[str] = set()
    models: list[str] = []
    alias_by_model: dict[str, str] = {}
    for alias, mid in alias_to_model.items():
        alias_by_model[mid] = alias
        if mid not in seen:
            seen.add(mid)
            models.append(mid)

    probes: dict[str, Any] = {}
    if not skip_probe:
        for alias, cfg in pool.items():
            ok, detail = probe_models(cfg)
            probes[alias] = {"ok": ok, "detail": detail}
            if not ok:
                notes.append(f"pool {alias} probe failed: {detail}")
        for rs in router_list:
            ok, detail = probe_models(rs.endpoint)
            probes[f"router:{rs.name}"] = {"ok": ok, "detail": detail}
            if not ok:
                notes.append(f"router {rs.name} probe failed: {detail}")

    ref_mid = ref_model
    if quality == "overlap":
        if not ref_mid:
            ref_mid = models[0]
        if ref_mid not in models and ref_mid not in alias_by_model:
            if ref_mid in alias_to_model:
                ref_mid = alias_to_model[ref_mid]
            else:
                notes.append(f"ref_model {ref_model!r} not in pool; overlap may be zero")

    question_ids = [str(q["id"]) for q in corpus]
    domains = {
        str(q["id"]): str(q["domain"]) for q in corpus if q.get("domain")
    }
    for q in corpus:
        qid = str(q["id"])
        if qid not in domains and "-" in qid:
            domains[qid] = qid.split("-", 1)[0]

    completions: dict[tuple[str, str], dict[str, Any]] = {}

    def _chat_pool(model: str, prompt: str) -> dict[str, Any]:
        key = (model, prompt)
        if key in completions:
            return completions[key]
        alias = alias_by_model.get(model)
        if alias is None:
            for a, m in alias_to_model.items():
                if m == model:
                    alias = a
                    break
        if alias is None or alias not in pool:
            rec = {
                "status": "error",
                "content": "",
                "tokens": 0,
                "error": "model not in pool",
            }
            completions[key] = rec
            return rec
        cfg = pool[alias]
        result = fn(cfg, model=model, prompt=prompt, max_tokens=max_tokens, temperature=0.0)
        if result.status != "ok":
            rec = {
                "status": "error",
                "content": "",
                "tokens": 0,
                "error": result.error,
                "latency_ms": result.latency_ms,
            }
        else:
            if result.completion_tokens and result.completion_tokens > 0:
                tok, src = result.completion_tokens, "usage"
            elif result.total_tokens and result.total_tokens > 0:
                tok, src = result.total_tokens, "usage"
            else:
                tok, src = estimate_tokens(result.content, None)
            rec = {
                "status": "ok",
                "content": result.content,
                "tokens": tok,
                "token_source": src,
                "latency_ms": result.latency_ms,
            }
        completions[key] = rec
        return rec

    ref_texts: dict[str, str] = {}
    if quality == "overlap" and ref_mid:
        for q in corpus:
            rec = _chat_pool(ref_mid, q["prompt"])
            ref_texts[str(q["id"])] = rec.get("content") or ""

    cells: dict[tuple[str, str], Cell] = {}
    for q in corpus:
        qid = str(q["id"])
        prompt = q["prompt"]
        expected = q.get("expected_model")
        for model in models:
            rec = _chat_pool(model, prompt)
            if rec["status"] != "ok":
                cells[(qid, model)] = Cell(
                    quality=0.0,
                    tokens=0,
                    cost_usd=0.0,
                    status="error",
                    detail={"error": rec.get("error")},
                )
                continue
            if quality == "label":
                qscore = label_score(model, expected)
                detail: dict[str, Any] = {
                    "quality_mode": "label",
                    "expected_model": expected,
                }
            else:
                qscore, detail = _score_cell_quality(
                    quality_mode=quality,
                    model=model,
                    content=rec["content"],
                    expected_model=expected,
                    ref_content=ref_texts.get(qid),
                    prompt=prompt,
                    judge_url=judge_url,
                    judge_model=judge_model,
                    judge_api_key=judge_api_key,
                    timeout_s=pool[alias_by_model[model]].timeout_s,
                    chat_fn=fn,
                )
                if quality == "judge" and detail.get("judge", {}).get("status") == "skipped":
                    cells[(qid, model)] = Cell(
                        quality=0.0,
                        tokens=int(rec["tokens"]),
                        cost_usd=cost_of(model, int(rec["tokens"]), price_table),
                        status="skipped",
                        detail=detail,
                    )
                    continue
            tokens = int(rec["tokens"])
            cells[(qid, model)] = Cell(
                quality=qscore,
                tokens=tokens,
                cost_usd=cost_of(model, tokens, price_table),
                status="ok",
                detail=detail,
            )

    matrix = RoutingMatrix(
        question_ids=question_ids,
        models=models,
        cells=cells,
        meta={"quality": quality, "eps": eps},
    )

    live_by_router: dict[str, dict[str, str | None]] = {}
    live_errors: list[dict[str, Any]] = []
    if not router_list:
        notes.append("no --router; live router policies omitted")
    for rs in router_list:
        picks: dict[str, str | None] = {}
        for q in corpus:
            qid = str(q["id"])
            result = fn(
                rs.endpoint,
                model=rs.entrypoint,
                prompt=q["prompt"],
                max_tokens=max_tokens,
                temperature=0.0,
            )
            if result.status != "ok":
                picks[qid] = None
                live_errors.append(
                    {
                        "router": rs.name,
                        "question_id": qid,
                        "status": "error",
                        "error": result.error,
                    }
                )
                continue
            raw_pick = result.picked_model(rs.pick_headers)
            resolved, err = resolve_pick(
                raw_pick,
                models=models,
                alias_to_model=alias_to_model,
                pick_map=pmap,
            )
            if err:
                live_errors.append(
                    {
                        "router": rs.name,
                        "question_id": qid,
                        "status": "error",
                        "error": err,
                    }
                )
            picks[qid] = resolved
        live_by_router[rs.name] = picks

    policy_rows: list[dict[str, Any]] = []
    for mid in models:
        policy_rows.append(
            evaluate_policy(matrix, always_policy(mid), policy_name=f"always_{mid}")
        )
    policy_rows.append(
        evaluate_policy(matrix, oracle_quality(matrix), policy_name="oracle_quality")
    )
    policy_rows.append(
        evaluate_policy(
            matrix, oracle_cost_optimal(matrix, eps=eps), policy_name="oracle_cost_optimal"
        )
    )
    if include_domain_knn:
        policy_rows.append(
            evaluate_policy(
                matrix, domain_router(matrix, domains), policy_name="domain_router"
            )
        )
        policy_rows.append(
            evaluate_policy(matrix, knn_router(matrix, k=3), policy_name="knn_router")
        )
    for rs in router_list:
        policy_rows.append(
            evaluate_policy(
                matrix,
                router_policy(live_by_router[rs.name]),
                policy_name=rs.name,
            )
        )

    ladder = analyse(policy_rows)
    ladder_summary = []
    picks_by_policy = {}
    for row in ladder:
        picks_by_policy[row["policy"]] = row.get("picks")
        ladder_summary.append({k: v for k, v in row.items() if k != "picks"})

    skipped = sum(1 for c in cells.values() if c.status == "skipped")

    return {
        "mode": "router_routing",
        "ci_fail": False,
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "quality": quality,
            "eps": eps,
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
            "pick_map": pmap,
            "ref_model": ref_mid,
            "models": models,
            "pool_aliases": alias_to_model,
        },
        "summary": {
            "questions": len(question_ids),
            "models": len(models),
            "cells_ok": sum(1 for c in cells.values() if c.status == "ok"),
            "cells_skipped": skipped,
            "cells_error": sum(1 for c in cells.values() if c.status == "error"),
            "live_router_errors": len(live_errors),
            "live_routers": len(router_list),
            "policies": len(ladder_summary),
        },
        "probes": probes,
        "matrix": matrix.to_serializable(),
        "ladder": ladder_summary,
        "picks": picks_by_policy,
        "live_errors": live_errors,
        "notes": notes,
    }
