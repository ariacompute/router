"""Perplexity-style rubric parse + weighted MET/UNMET aggregation."""

from __future__ import annotations

import json
from typing import Any

from .label import keyword_hit_score

# Canonical four axes (Perplexity DRACO-shaped).
AXES = (
    "factual-accuracy",
    "breadth-and-depth",
    "presentation",
    "citation",
)

# Aliases → canonical
_AXIS_ALIASES: dict[str, str] = {
    "factual_accuracy": "factual-accuracy",
    "factual-accuracy": "factual-accuracy",
    "factual": "factual-accuracy",
    "accuracy": "factual-accuracy",
    "breadth_and_depth": "breadth-and-depth",
    "breadth-and-depth": "breadth-and-depth",
    "breadth": "breadth-and-depth",
    "depth": "breadth-and-depth",
    "presentation": "presentation",
    "clarity": "presentation",
    "citation": "citation",
    "citations": "citation",
    "sources": "citation",
}


def normalize_axis(name: str) -> str:
    key = (name or "").strip().lower().replace(" ", "-").replace("_", "-")
    # re-map common underscore forms after replace
    key2 = key.replace("-", "_")
    if key in _AXIS_ALIASES:
        return _AXIS_ALIASES[key]
    if key2 in _AXIS_ALIASES:
        return _AXIS_ALIASES[key2]
    # try with underscores restored from original
    raw = (name or "").strip().lower().replace(" ", "_")
    if raw in _AXIS_ALIASES:
        return _AXIS_ALIASES[raw]
    return key if key in AXES else key


def parse_rubric(answer: Any) -> dict[str, Any]:
    """Parse ``answer`` field: JSON string or object → sections/criteria.

    Accepted shapes:
    - ``{"sections": [{"name": "...", "weight": 0.25, "criteria": [{"text": "...", "weight": 1}]}]}``
    - ``{"factual-accuracy": {"weight": 0.25, "criteria": [...]}, ...}``
    - nested list of criteria strings
    """
    if isinstance(answer, str):
        answer = json.loads(answer)
    if not isinstance(answer, dict):
        raise ValueError(f"rubric answer must be object or JSON string, got {type(answer).__name__}")

    sections: list[dict[str, Any]] = []
    if "sections" in answer and isinstance(answer["sections"], list):
        for sec in answer["sections"]:
            sections.append(_normalize_section(sec))
    else:
        # axis-keyed object
        for k, v in answer.items():
            if k in ("expected_hits", "expected_model", "id", "domain"):
                continue
            if isinstance(v, dict):
                sec = dict(v)
                sec.setdefault("name", k)
                sections.append(_normalize_section(sec))
            elif isinstance(v, list):
                sections.append(
                    _normalize_section({"name": k, "weight": 1.0, "criteria": v})
                )

    if not sections:
        raise ValueError("rubric has no sections/criteria")

    # renormalize section weights if all missing
    total_w = sum(float(s["weight"]) for s in sections)
    if total_w <= 0:
        for s in sections:
            s["weight"] = 1.0 / len(sections)
    else:
        for s in sections:
            s["weight"] = float(s["weight"]) / total_w

    return {"sections": sections, "axes": [s["axis"] for s in sections]}


def _normalize_section(sec: dict[str, Any]) -> dict[str, Any]:
    name = str(sec.get("name") or sec.get("axis") or "unknown")
    axis = normalize_axis(str(sec.get("axis") or name))
    weight = float(sec.get("weight", 1.0))
    raw_crit = sec.get("criteria") or sec.get("items") or []
    criteria: list[dict[str, Any]] = []
    if isinstance(raw_crit, list):
        for c in raw_crit:
            if isinstance(c, str):
                criteria.append({"text": c, "weight": 1.0})
            elif isinstance(c, dict):
                criteria.append(
                    {
                        "text": str(c.get("text") or c.get("criterion") or c.get("description") or ""),
                        "weight": float(c.get("weight", 1.0)),
                    }
                )
    if not criteria:
        criteria = [{"text": name, "weight": 1.0}]
    cw = sum(float(c["weight"]) for c in criteria)
    if cw <= 0:
        for c in criteria:
            c["weight"] = 1.0 / len(criteria)
    else:
        for c in criteria:
            c["weight"] = float(c["weight"]) / cw
    return {"name": name, "axis": axis, "weight": weight, "criteria": criteria}


def aggregate_rubric(
    sections: list[dict[str, Any]],
    met_flags: list[list[bool | None]],
) -> dict[str, Any]:
    """Weighted aggregate given per-section per-criterion MET flags.

    ``met_flags[i][j]`` corresponds to ``sections[i]["criteria"][j]``.
    ``None`` criteria are ignored in that section's local weight renormalization.
    """
    axis_scores: dict[str, list[float]] = {a: [] for a in AXES}
    section_scores: list[dict[str, Any]] = []
    overall_num = 0.0
    overall_den = 0.0

    for sec, flags in zip(sections, met_flags):
        crits = sec["criteria"]
        if len(flags) != len(crits):
            raise ValueError("met_flags length mismatch criteria")
        local_num = 0.0
        local_den = 0.0
        detail = []
        for c, m in zip(crits, flags):
            detail.append({"text": c["text"], "met": m, "weight": c["weight"]})
            if m is None:
                continue
            local_den += float(c["weight"])
            if m:
                local_num += float(c["weight"])
        sec_score = (local_num / local_den) if local_den > 0 else 0.0
        section_scores.append(
            {
                "name": sec["name"],
                "axis": sec["axis"],
                "weight": sec["weight"],
                "score": sec_score,
                "criteria": detail,
            }
        )
        axis = sec["axis"]
        if axis not in axis_scores:
            axis_scores[axis] = []
        axis_scores[axis].append(sec_score)
        overall_num += sec_score * float(sec["weight"])
        overall_den += float(sec["weight"])

    axes_out: dict[str, float] = {}
    for a in AXES:
        vals = axis_scores.get(a) or []
        axes_out[a] = sum(vals) / len(vals) if vals else 0.0
    # include any extra axes
    for a, vals in axis_scores.items():
        if a not in axes_out and vals:
            axes_out[a] = sum(vals) / len(vals)

    overall = (overall_num / overall_den) if overall_den > 0 else 0.0
    return {
        "score": overall,
        "axes": axes_out,
        "sections": section_scores,
    }


def score_label_rubric(
    completion: str,
    rubric: dict[str, Any],
    expected_hits: list[str] | None = None,
) -> dict[str, Any]:
    """Offline label mode: keyword hits drive MET; optional expected_hits for overall.

    If ``expected_hits`` provided, overall score is keyword_hit_score and each
    criterion is MET iff any hit substring appears in criterion text **and**
    in completion, else MET if the criterion's own keywords (split) appear.
    Simpler path for tiny fixtures: criterion MET if any word longer than 3
    chars from criterion text appears in completion; section aggregates as usual.
    When ``expected_hits`` is set, also blend: criterion MET if any expected_hit
    appears in completion (shared signal for CI).
    """
    sections = rubric["sections"]
    met_flags: list[list[bool | None]] = []
    lower = (completion or "").lower()

    for sec in sections:
        flags: list[bool | None] = []
        for c in sec["criteria"]:
            text = c["text"]
            if expected_hits:
                # MET if any expected hit appears in the answer
                met = any(h.lower() in lower for h in expected_hits if h)
            else:
                tokens = [t for t in text.lower().replace(",", " ").split() if len(t) > 3]
                if not tokens:
                    met = bool(lower.strip())
                else:
                    met = any(t in lower for t in tokens)
            flags.append(met)
        met_flags.append(flags)

    agg = aggregate_rubric(sections, met_flags)
    if expected_hits:
        # Prefer explicit hit rate as primary score for tiny CI
        agg["score"] = keyword_hit_score(completion, expected_hits)
        agg["label_mode"] = "expected_hits"
    else:
        agg["label_mode"] = "criterion_keywords"
    return agg
