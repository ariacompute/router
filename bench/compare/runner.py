"""MCQ compare runner: accuracy + E2E latency + tokens (vs vLLM SR narrative)."""

from __future__ import annotations

import json
import statistics
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
from ..router_targets import RouterSpec
from .grade import format_mcq_prompt, grade_answer


def load_compare_corpus(path: str | Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    text = Path(path).read_text(encoding="utf-8")
    for i, line in enumerate(text.splitlines()):
        line = line.strip()
        if not line:
            continue
        obj = json.loads(line)
        if not isinstance(obj, dict):
            raise ValueError(f"line {i+1}: expected object")
        if "question" not in obj and "prompt" not in obj:
            raise ValueError(f"line {i+1}: missing question")
        if "answer" not in obj:
            raise ValueError(f"line {i+1}: missing answer")
        obj.setdefault("id", f"m{i}")
        if "question" not in obj:
            obj["question"] = obj["prompt"]
        rows.append(obj)
    return rows


def _percentile(sorted_vals: Sequence[float], p: float) -> float:
    if not sorted_vals:
        return 0.0
    if len(sorted_vals) == 1:
        return float(sorted_vals[0])
    k = (len(sorted_vals) - 1) * (p / 100.0)
    f = int(k)
    c = min(f + 1, len(sorted_vals) - 1)
    if f == c:
        return float(sorted_vals[f])
    return float(sorted_vals[f] + (sorted_vals[c] - sorted_vals[f]) * (k - f))


def _agg_latencies(vals: list[float]) -> dict[str, float]:
    if not vals:
        return {"mean_ms": 0.0, "p50_ms": 0.0, "p95_ms": 0.0, "n": 0}
    s = sorted(vals)
    return {
        "mean_ms": float(statistics.fmean(s)),
        "p50_ms": _percentile(s, 50),
        "p95_ms": _percentile(s, 95),
        "n": len(s),
    }


def run_compare(
    *,
    corpus: list[dict[str, Any]],
    pool: Mapping[str, EndpointConfig],
    model_ids: Mapping[str, str],
    routers: Sequence[RouterSpec] | None = None,
    prices: Mapping[str, float] | None = None,
    max_tokens: int = 64,
    skip_probe: bool = False,
    chat_fn: ChatFn | None = None,
) -> dict[str, Any]:
    router_list = list(routers or [])
    if not pool and not router_list:
        raise ValueError("need --pool and/or --router")

    fn = chat_fn or chat_completion
    price_table = dict(prices) if prices is not None else load_prices(None)
    notes: list[str] = []
    probes: dict[str, Any] = {}
    alias_to_model = {a: model_ids.get(a, a) for a in pool.keys()}

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

    systems: list[tuple[str, str, EndpointConfig, list[str] | None]] = []
    for alias, cfg in pool.items():
        mid = alias_to_model[alias]
        systems.append((f"always_{mid}", mid, cfg, None))
    for rs in router_list:
        systems.append((rs.name, rs.entrypoint, rs.endpoint, rs.pick_headers))
    if not router_list:
        notes.append("no --router; live router systems omitted")

    results: list[dict[str, Any]] = []
    by_system: dict[str, list[dict[str, Any]]] = {n: [] for n, _, _, _ in systems}

    for item in corpus:
        tid = str(item["id"])
        question = str(item["question"])
        choices = item.get("choices")
        if isinstance(choices, str):
            choices = [choices]
        gold = item.get("answer")
        category = item.get("category") or item.get("domain") or "general"
        prompt = format_mcq_prompt(question, choices if isinstance(choices, list) else None)

        for sys_name, model, cfg, pick_headers in systems:
            chat = fn(
                cfg, model=model, prompt=prompt, max_tokens=max_tokens, temperature=0.0
            )
            row: dict[str, Any] = {
                "id": tid,
                "category": category,
                "system": sys_name,
                "request_model": model,
            }
            if chat.status != "ok":
                row.update(
                    {
                        "status": "error",
                        "error": chat.error,
                        "is_correct": False,
                        "latency_ms": chat.latency_ms,
                    }
                )
                results.append(row)
                continue

            if pick_headers is not None:
                row["routed_model"] = chat.picked_model(pick_headers)
            graded = grade_answer(
                completion=chat.content,
                gold=gold,
                choices=choices if isinstance(choices, list) else None,
            )
            pt = chat.prompt_tokens
            ct = chat.completion_tokens
            if ct is None:
                ct, _ = estimate_tokens(chat.content, None)
            tt = chat.total_tokens
            if tt is None and isinstance(pt, int):
                tt = pt + int(ct)
            elif tt is None:
                tt = int(ct)
            cost_model = row.get("routed_model") or model
            row.update(
                {
                    "status": "ok",
                    "latency_ms": chat.latency_ms,
                    "prompt_tokens": pt,
                    "completion_tokens": int(ct),
                    "total_tokens": int(tt) if tt is not None else int(ct),
                    "cost_usd": cost_of(str(cost_model), int(ct), price_table),
                    "preview": (chat.content or "")[:200],
                    **graded,
                }
            )
            results.append(row)
            by_system[sys_name].append(row)

    system_summaries: list[dict[str, Any]] = []
    always_acc: dict[str, float] = {}
    always_lat: dict[str, float] = {}
    for sys_name, _, _, _ in systems:
        rows = by_system.get(sys_name) or []
        n_ok = len(rows)
        n_correct = sum(1 for r in rows if r.get("is_correct"))
        acc = (n_correct / n_ok) if n_ok else 0.0
        lats = [float(r["latency_ms"]) for r in rows if "latency_ms" in r]
        toks_c = [float(r.get("completion_tokens") or 0) for r in rows]
        toks_p = [float(r.get("prompt_tokens") or 0) for r in rows if r.get("prompt_tokens") is not None]
        toks_t = [float(r.get("total_tokens") or 0) for r in rows]
        costs = [float(r.get("cost_usd") or 0) for r in rows]
        summary = {
            "system": sys_name,
            "accuracy": acc,
            "n_correct": n_correct,
            "n_ok": n_ok,
            "n_total": len(corpus),
            "latency_ms": _agg_latencies(lats),
            "avg_prompt_tokens": float(statistics.fmean(toks_p)) if toks_p else None,
            "avg_completion_tokens": float(statistics.fmean(toks_c)) if toks_c else 0.0,
            "avg_total_tokens": float(statistics.fmean(toks_t)) if toks_t else 0.0,
            "avg_cost_usd": float(statistics.fmean(costs)) if costs else 0.0,
        }
        system_summaries.append(summary)
        if sys_name.startswith("always_"):
            always_acc[sys_name] = acc
            always_lat[sys_name] = summary["latency_ms"]["mean_ms"]

    best_always_name = None
    best_always_acc = None
    if always_acc:
        best_always_name = max(always_acc, key=lambda k: always_acc[k])
        best_always_acc = always_acc[best_always_name]
    best_always_lat = always_lat.get(best_always_name) if best_always_name else None

    for s in system_summaries:
        if best_always_acc is not None:
            s["accuracy_delta_vs_best_always"] = float(s["accuracy"]) - best_always_acc
        else:
            s["accuracy_delta_vs_best_always"] = None
        mean_lat = s["latency_ms"]["mean_ms"]
        if best_always_lat and best_always_lat > 0:
            s["latency_ratio_vs_best_always"] = mean_lat / best_always_lat
        else:
            s["latency_ratio_vs_best_always"] = None

    return {
        "mode": "router_compare",
        "ci_fail": False,
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "tasks": len(corpus),
            "routers": [
                {
                    "name": rs.name,
                    "base_url": rs.endpoint.base_url,
                    "entrypoint": rs.entrypoint,
                    "pick_headers": rs.pick_headers,
                }
                for rs in router_list
            ],
            "systems": [n for n, _, _, _ in systems],
            "max_tokens": max_tokens,
        },
        "summary": {
            "tasks": len(corpus),
            "systems": len(systems),
            "results_ok": sum(1 for r in results if r.get("status") == "ok"),
            "results_error": sum(1 for r in results if r.get("status") == "error"),
            "best_always": best_always_name,
            "best_always_accuracy": best_always_acc,
        },
        "probes": probes,
        "systems": system_summaries,
        "results": results,
        "notes": notes,
    }
