"""Score research completions against rubrics."""

from __future__ import annotations

from typing import Any

from ..http_client import ChatFn
from ..quality.judge import judge_criterion
from ..quality.rubric import AXES, aggregate_rubric, parse_rubric, score_label_rubric


def score_research_item(
    *,
    problem: str,
    completion: str,
    answer: Any,
    quality: str = "label",
    expected_hits: list[str] | None = None,
    judge_url: str | None = None,
    judge_model: str = "judge",
    judge_api_key: str = "",
    timeout_s: float = 120.0,
    chat_fn: ChatFn | None = None,
) -> dict[str, Any]:
    """Return score dict with axes breakdown."""
    rubric = parse_rubric(answer)
    sections = rubric["sections"]

    if quality == "label":
        agg = score_label_rubric(completion, rubric, expected_hits=expected_hits)
        return {
            "status": "ok",
            "score": agg["score"],
            "axes": agg["axes"],
            "sections": agg["sections"],
            "quality_mode": "label",
        }

    if quality == "overlap":
        # Not primary for research; treat as label keyword fallback
        agg = score_label_rubric(completion, rubric, expected_hits=expected_hits)
        return {
            "status": "ok",
            "score": agg["score"],
            "axes": agg["axes"],
            "sections": agg["sections"],
            "quality_mode": "overlap_as_label",
            "note": "research overlap falls back to label/rubric keywords",
        }

    if quality == "judge":
        if not judge_url:
            return {
                "status": "skipped",
                "score": 0.0,
                "axes": {a: 0.0 for a in AXES},
                "reason": "no --judge-url; Mode B judge unavailable",
                "quality_mode": "judge",
            }
        met_flags: list[list[bool | None]] = []
        for sec in sections:
            flags: list[bool | None] = []
            for c in sec["criteria"]:
                j = judge_criterion(
                    problem=problem,
                    completion=completion,
                    criterion=c["text"],
                    judge_url=judge_url,
                    judge_model=judge_model,
                    judge_api_key=judge_api_key,
                    timeout_s=timeout_s,
                    chat_fn=chat_fn,
                )
                if j["status"] == "skipped":
                    return {
                        "status": "skipped",
                        "score": 0.0,
                        "axes": {a: 0.0 for a in AXES},
                        "reason": j.get("reason"),
                        "quality_mode": "judge",
                    }
                if j["status"] != "ok":
                    flags.append(None)
                else:
                    flags.append(bool(j["met"]))
            met_flags.append(flags)
        agg = aggregate_rubric(sections, met_flags)
        return {
            "status": "ok",
            "score": agg["score"],
            "axes": agg["axes"],
            "sections": agg["sections"],
            "quality_mode": "judge",
        }

    raise ValueError(f"unknown quality mode {quality!r}")


def mean_axes(rows: list[dict[str, Any]]) -> dict[str, float]:
    sums = {a: 0.0 for a in AXES}
    counts = {a: 0 for a in AXES}
    for r in rows:
        axes = r.get("axes") or {}
        for a in AXES:
            if a in axes and isinstance(axes[a], (int, float)):
                sums[a] += float(axes[a])
                counts[a] += 1
    return {a: (sums[a] / counts[a] if counts[a] else 0.0) for a in AXES}
