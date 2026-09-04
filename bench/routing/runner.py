"""Routing bench runner: corpus → matrix → policies → report."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping

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


def run_routing(
    *,
    corpus: list[dict[str, Any]],
    pool: Mapping[str, EndpointConfig],
    model_ids: Mapping[str, str],
    quality: str = "label",
    router: EndpointConfig | None = None,
    entrypoint: str = "aria/semantic-auto",
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

    price_table = dict(prices) if prices is not None else load_prices(None)
    fn = chat_fn or chat_completion
    notes: list[str] = []

    # Resolve pool alias -> served model id
    alias_to_model = {
        alias: model_ids.get(alias, alias) for alias in pool.keys()
    }
    models = [alias_to_model[a] for a in pool.keys()]
    # unique preserve order
    seen: set[str] = set()
    models_u: list[str] = []
    alias_by_model: dict[str, str] = {}
    for alias, mid in alias_to_model.items():
        alias_by_model[mid] = alias
        if mid not in seen:
            seen.add(mid)
            models_u.append(mid)
    models = models_u

    probes: dict[str, Any] = {}
    if not skip_probe:
        for alias, cfg in pool.items():
            ok, detail = probe_models(cfg)
            probes[alias] = {"ok": ok, "detail": detail}
            if not ok:
                notes.append(f"pool {alias} probe failed: {detail}")
        if router is not None:
            ok, detail = probe_models(router)
            probes["router"] = {"ok": ok, "detail": detail}
            if not ok:
                notes.append(f"router probe failed: {detail}")

    # For overlap: need ref completions per question
    ref_mid = ref_model
    if quality == "overlap":
        if not ref_mid:
            ref_mid = models[0]
        if ref_mid not in models and ref_mid not in alias_by_model:
            # allow alias
            if ref_mid in alias_to_model:
                ref_mid = alias_to_model[ref_mid]
            else:
                notes.append(f"ref_model {ref_model!r} not in pool; overlap may be zero")

    question_ids = [str(q["id"]) for q in corpus]
    domains = {
        str(q["id"]): str(q["domain"])
        for q in corpus
        if q.get("domain")
    }
    # also derive from id prefix
    for q in corpus:
        qid = str(q["id"])
        if qid not in domains and "-" in qid:
            domains[qid] = qid.split("-", 1)[0]

    # Chat cache: (alias_or_router, model, prompt) — we key by model id
    completions: dict[tuple[str, str], dict[str, Any]] = {}

    def _chat_pool(model: str, prompt: str) -> dict[str, Any]:
        key = (model, prompt)
        if key in completions:
            return completions[key]
        alias = alias_by_model.get(model)
        if alias is None:
            # find alias whose model_ids maps to model
            for a, m in alias_to_model.items():
                if m == model:
                    alias = a
                    break
        if alias is None or alias not in pool:
            rec = {"status": "error", "content": "", "tokens": 0, "error": "model not in pool"}
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
            tok, src = estimate_tokens(
                result.content,
                result.total_tokens
                if result.total_tokens
                else (
                    (result.prompt_tokens or 0) + (result.completion_tokens or 0)
                    if result.prompt_tokens or result.completion_tokens
                    else result.completion_tokens
                ),
            )
            # Prefer completion tokens for cost if available
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

    # Prefetch ref completions for overlap
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
            # For label mode, quality is whether THIS model is the expected one
            # (cell quality = 1 if model==expected else 0), matching ADR decision accuracy.
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

    # Live router picks
    live_picks: dict[str, str | None] = {}
    live_errors: list[dict[str, Any]] = []
    if router is not None:
        for q in corpus:
            qid = str(q["id"])
            result = fn(
                router,
                model=entrypoint,
                prompt=q["prompt"],
                max_tokens=max_tokens,
                temperature=0.0,
            )
            if result.status != "ok":
                live_picks[qid] = None
                live_errors.append(
                    {"question_id": qid, "status": "error", "error": result.error}
                )
                continue
            picked = result.router_model
            if picked and picked not in models:
                # try match via alias map values / keys
                if picked in alias_to_model:
                    picked = alias_to_model[picked]
                elif picked in alias_by_model:
                    pass
                else:
                    live_errors.append(
                        {
                            "question_id": qid,
                            "status": "error",
                            "error": f"pick {picked!r} not in pool {models}",
                        }
                    )
                    live_picks[qid] = picked  # still record; evaluate will error
                    continue
            live_picks[qid] = picked
    else:
        notes.append("no --router; aria_router policy omitted")

    policy_rows: list[dict[str, Any]] = []
    for mid in models:
        row = evaluate_policy(matrix, always_policy(mid), policy_name=f"always_{mid}")
        policy_rows.append(row)

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
    if router is not None:
        policy_rows.append(
            evaluate_policy(
                matrix, router_policy(live_picks), policy_name="aria_router"
            )
        )

    ladder = analyse(policy_rows)
    # Strip picks from ladder summary for compactness? Keep in report under picks
    ladder_summary = []
    picks_by_policy = {}
    for row in ladder:
        picks_by_policy[row["policy"]] = row.get("picks")
        summary_row = {k: v for k, v in row.items() if k != "picks"}
        ladder_summary.append(summary_row)

    skipped = sum(1 for c in cells.values() if c.status == "skipped")
    errors = sum(1 for c in cells.values() if c.status == "error") + len(live_errors)

    return {
        "mode": "router_routing",
        "ci_fail": False,
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "quality": quality,
            "eps": eps,
            "entrypoint": entrypoint,
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
            "policies": len(ladder_summary),
        },
        "probes": probes,
        "matrix": matrix.to_serializable(),
        "ladder": ladder_summary,
        "picks": picks_by_policy,
        "live_errors": live_errors,
        "notes": notes,
    }
