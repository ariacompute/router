"""Live-router accuracy: POST chat completions, score pick vs expected_model."""

from __future__ import annotations

from collections import defaultdict
from datetime import datetime, timezone
from typing import Any, Mapping, Sequence

from ..http_client import ChatFn, EndpointConfig, chat_completion
from ..quality.label import label_score
from ..router_targets import resolve_pick
from ..routing.runner import load_routing_corpus

# Logical → upstream (default Gateway triad).
_DEFAULT_PICK_MAP = {
    "ariacompute/ariamodel-small": "glm-5.3-flash",
    "ariacompute/ariamodel-mid": "glm-5.3",
    "ariacompute/ariamodel-large": "deepseek-v4-pro",
}

_UPSTREAM_MODELS = ("glm-5.3-flash", "glm-5.3", "deepseek-v4-pro")

_TIER_OF = {
    "glm-5.3-flash": "small",
    "glm-5.3": "mid",
    "deepseek-v4-pro": "large",
}


def _tier(model: str | None) -> str:
    if not model:
        return "unknown"
    return _TIER_OF.get(model, model)


def run_routing_accuracy(
    *,
    corpus: list[dict[str, Any]] | str,
    router: EndpointConfig,
    entrypoint: str = "ariacompute/semantic-auto",
    pick_headers: Sequence[str] | None = None,
    pick_map: Mapping[str, str] | None = None,
    max_tokens: int = 16,
    chat_fn: ChatFn | None = None,
) -> dict[str, Any]:
    """Score each prompt by comparing mapped router pick to ``expected_model``.

    Does not call upstream pool models — only the live router.
    """
    if isinstance(corpus, str):
        items = load_routing_corpus(corpus)
    else:
        items = list(corpus)

    headers = list(pick_headers) if pick_headers is not None else ["x-aria-router-model"]
    pmap = dict(_DEFAULT_PICK_MAP)
    if pick_map:
        pmap.update(pick_map)
    models = list(_UPSTREAM_MODELS)
    alias_to_model = {
        "small": "glm-5.3-flash",
        "mid": "glm-5.3",
        "large": "deepseek-v4-pro",
    }
    fn = chat_fn or chat_completion

    rows: list[dict[str, Any]] = []
    errors: list[dict[str, Any]] = []
    confusion: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    by_expected: dict[str, dict[str, int]] = defaultdict(lambda: {"n": 0, "correct": 0})

    for item in items:
        qid = str(item.get("id") or "")
        prompt = str(item["prompt"])
        expected = item.get("expected_model")
        expected_s = str(expected).strip() if expected else None
        exp_tier = _tier(expected_s)

        result = fn(
            router,
            model=entrypoint,
            prompt=prompt,
            max_tokens=max_tokens,
            temperature=0.0,
        )
        if result.status != "ok":
            errors.append(
                {
                    "question_id": qid,
                    "status": "error",
                    "error": result.error,
                }
            )
            rows.append(
                {
                    "id": qid,
                    "expected_model": expected_s,
                    "expected_tier": exp_tier,
                    "routed_model_raw": None,
                    "routed_model": None,
                    "correct": False,
                    "status": "error",
                    "error": result.error,
                    "latency_ms": result.latency_ms,
                }
            )
            by_expected[exp_tier]["n"] += 1
            continue

        raw_pick = result.picked_model(list(headers))
        resolved, err = resolve_pick(
            raw_pick,
            models=models,
            alias_to_model=alias_to_model,
            pick_map=pmap,
        )
        if err:
            errors.append(
                {
                    "question_id": qid,
                    "status": "error",
                    "error": err,
                    "routed_model_raw": raw_pick,
                }
            )
            rows.append(
                {
                    "id": qid,
                    "expected_model": expected_s,
                    "expected_tier": exp_tier,
                    "routed_model_raw": raw_pick,
                    "routed_model": resolved,
                    "correct": False,
                    "status": "error",
                    "error": err,
                    "latency_ms": result.latency_ms,
                }
            )
            by_expected[exp_tier]["n"] += 1
            continue

        score = label_score(resolved, expected_s)
        correct = score >= 1.0
        got_tier = _tier(resolved)
        confusion[exp_tier][got_tier] += 1
        by_expected[exp_tier]["n"] += 1
        if correct:
            by_expected[exp_tier]["correct"] += 1

        rows.append(
            {
                "id": qid,
                "expected_model": expected_s,
                "expected_tier": exp_tier,
                "routed_model_raw": raw_pick,
                "routed_model": resolved,
                "routed_tier": got_tier,
                "correct": correct,
                "status": "ok",
                "latency_ms": result.latency_ms,
                "intent": item.get("intent"),
            }
        )

    n_total = len(items)
    n_scored = sum(1 for r in rows if r["status"] == "ok")
    n_correct = sum(1 for r in rows if r.get("correct"))
    overall = (n_correct / n_scored) if n_scored else 0.0

    recall: dict[str, float] = {}
    for tier, st in sorted(by_expected.items()):
        recall[tier] = (st["correct"] / st["n"]) if st["n"] else 0.0

    confusion_table = {
        exp: dict(sorted(got.items())) for exp, got in sorted(confusion.items())
    }

    return {
        "mode": "routing_accuracy",
        "ci_fail": False,
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "entrypoint": entrypoint,
            "base_url": router.base_url,
            "pick_headers": headers,
            "pick_map": pmap,
            "questions": n_total,
        },
        "summary": {
            "questions": n_total,
            "scored": n_scored,
            "correct": n_correct,
            "errors": len(errors),
            "overall_accuracy": overall,
            "recall_by_expected_tier": recall,
            "by_expected_tier": {k: dict(v) for k, v in sorted(by_expected.items())},
        },
        "confusion": confusion_table,
        "results": rows,
        "errors": errors,
    }
