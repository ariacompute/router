"""Offline policy-consistency recomputation (report-only).

A *decision record* is a saved routing decision (e.g. a replay record or an
accuracy result row) plus the eligible pool / label / evidence metadata of that
request. This module never calls a router: it recomputes deterministic policies
from the saved data and reports how often the recorded route agrees with them.

Report-only: ``ci_fail`` is always False. Missing data is reported as
``has_data = False`` / ``status = insufficient_evidence``, never as zero.
"""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

POLICIES: tuple[str, ...] = (
    "cheapest",
    "strongest",
    "expected_label",
    "recorded_confidence",
)

_TIER_RANK = {"small": 0, "mid": 1, "medium": 1, "large": 2, "unknown": 3}


def tier_of(model: str | None) -> str:
    """Best-effort competence tier of a logical / upstream model id."""
    if not model:
        return "unknown"
    name = str(model).lower()
    for token in ("small", "flash", "mid", "large", "pro"):
        if token in name:
            return {"small": "small", "flash": "small", "mid": "mid", "large": "large", "pro": "large"}[token]
    return "unknown"


def load_decision_records(path: str | Path) -> list[dict[str, Any]]:
    """Load ``.json`` (list / ``{"records": [...]}``) or ``.jsonl`` records.

    Malformed lines raise ``ValueError``; callers surface a bad-CLI exit code.
    """
    p = Path(path)
    suffix = p.suffix.lower()
    if suffix in (".jsonl", ".ndjson"):
        out: list[dict[str, Any]] = []
        for i, line in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError as e:
                raise ValueError(f"{p}:{i}: invalid JSON: {e}") from e
            if not isinstance(rec, dict):
                raise ValueError(f"{p}:{i}: decision record must be an object")
            out.append(rec)
        return out
    data = json.loads(p.read_text(encoding="utf-8"))
    if isinstance(data, dict):
        data = data.get("records", [data])
    if not isinstance(data, list):
        raise ValueError(f"{p}: expected a list of decision records")
    for i, rec in enumerate(data, 1):
        if not isinstance(rec, dict):
            raise ValueError(f"{p}:{i}: decision record must be an object")
    return data


def recorded_model(rec: Mapping[str, Any]) -> str | None:
    decision = rec.get("decision")
    if isinstance(decision, Mapping):
        model = decision.get("model")
    else:
        model = rec.get("routed_model") or rec.get("model")
    return str(model) if model else None


def eligible_models(rec: Mapping[str, Any]) -> list[str]:
    raw = rec.get("eligible") or rec.get("eligible_models") or []
    if isinstance(raw, Mapping):
        raw = list(raw.keys())
    if isinstance(raw, str):
        raw = [raw]
    if not isinstance(raw, Sequence):
        return []
    return [str(m) for m in raw]


def _pool(rec: Mapping[str, Any]) -> list[str]:
    pool = eligible_models(rec)
    if pool:
        return pool
    model = recorded_model(rec)
    return [model] if model else []


def recommend(rec: Mapping[str, Any], policy: str) -> tuple[str | None, str]:
    """Deterministic offline recommendation. Returns ``(model, reason)``."""
    if policy not in POLICIES:
        raise ValueError(f"unknown policy {policy!r}; choose from {', '.join(POLICIES)}")
    pool = _pool(rec)
    if not pool:
        return None, "no_eligible_models"
    if policy == "cheapest":
        pick = min(pool, key=lambda m: (_TIER_RANK[tier_of(m)], m))
        return pick, "lowest competence tier in eligible pool"
    if policy == "strongest":
        pick = sorted(pool, key=lambda m: (_TIER_RANK[tier_of(m)], m))[-1]
        return pick, "highest competence tier in eligible pool"
    if policy == "expected_label":
        expected = rec.get("expected_model") or rec.get("expected_tier")
        expected_s = str(expected) if expected else None
        if expected_s:
            for m in pool:
                if m == expected_s or tier_of(m) == expected_s:
                    return m, "saved label / expected tier"
        return None, "no matching saved label in eligible pool"
    # recorded_confidence: high confidence keeps the recorded route, low
    # confidence falls back to the cheapest eligible model.
    decision = rec.get("decision")
    conf = decision.get("confidence") if isinstance(decision, Mapping) else rec.get("confidence")
    try:
        conf_f = float(conf) if conf is not None else 1.0
    except (TypeError, ValueError):
        conf_f = 1.0
    if conf_f >= 0.5:
        recorded = recorded_model(rec)
        if recorded and recorded in pool:
            return recorded, f"confidence {conf_f:.2f} >= 0.5"
    return min(pool, key=lambda m: (_TIER_RANK[tier_of(m)], m)), "confidence below floor"


def _evidence_usable(
    rec: Mapping[str, Any],
    plan_version: str | None,
    rubric_version: str | None,
    task_category: str | None,
    entrypoint: str | None,
) -> str | None:
    """Return None when usable, else a stable exclusion reason."""
    if plan_version is not None and rec.get("plan_version") != plan_version:
        return "plan_version_mismatch"
    if rubric_version is not None and rec.get("rubric_version") != rubric_version:
        return "rubric_version_mismatch"
    if task_category is not None and rec.get("task_category") != task_category:
        return "task_category_mismatch"
    if entrypoint is not None and rec.get("entrypoint") != entrypoint:
        return "entrypoint_mismatch"
    if recorded_model(rec) is None:
        return "missing_recorded_decision"
    return None


def run_policy_consistency(
    *,
    records: Sequence[Mapping[str, Any]],
    policies: Sequence[str] = ("cheapest", "expected_label"),
    records_path: str | None = None,
    plan_version: str | None = None,
    rubric_version: str | None = None,
    task_category: str | None = None,
    entrypoint: str | None = None,
    min_evidence: int = 1,
    generated_at: str | None = None,
) -> dict[str, Any]:
    """Recompute policies offline and report agreement with recorded routes."""
    for pol in policies:
        if pol not in POLICIES:
            raise ValueError(f"unknown policy {pol!r}; choose from {', '.join(POLICIES)}")
    if min_evidence < 0:
        raise ValueError("min_evidence must be >= 0")

    usable: list[Mapping[str, Any]] = []
    errors: list[dict[str, Any]] = []
    for rec in records:
        rid = str(rec.get("id") or rec.get("question_id") or len(errors))
        reason = _evidence_usable(
            rec, plan_version, rubric_version, task_category, entrypoint
        )
        if reason:
            errors.append({"id": rid, "status": "excluded", "reason": reason})
        else:
            usable.append(rec)

    results: list[dict[str, Any]] = []
    per_policy: dict[str, dict[str, Any]] = {}
    total_comparable = 0
    total_agree = 0
    for pol in policies:
        matches = 0
        comparable = 0
        mismatches: list[dict[str, Any]] = []
        for rec in usable:
            pick, reason = recommend(rec, pol)
            model = recorded_model(rec)
            comparable_flag = pick is not None
            agree = comparable_flag and pick == model
            if comparable_flag:
                comparable += 1
                if agree:
                    matches += 1
                else:
                    mismatches.append(
                        {
                            "id": rec.get("id") or rec.get("question_id"),
                            "recorded": model,
                            "recomputed": pick,
                            "recomputed_tier": tier_of(pick),
                            "reason": reason,
                        }
                    )
            results.append(
                {
                    "id": rec.get("id") or rec.get("question_id"),
                    "policy": pol,
                    "recorded": model,
                    "recomputed": pick,
                    "recomputed_tier": tier_of(pick),
                    "comparable": comparable_flag,
                    "agrees": agree,
                    "reason": reason,
                }
            )
        total_comparable += comparable
        total_agree += matches
        per_policy[pol] = {
            "comparable": comparable,
            "agreements": matches,
            "consistency": (matches / comparable) if comparable else None,
            "mismatches": mismatches,
        }

    has_data = total_comparable > 0
    status = (
        "ready"
        if has_data and len(usable) >= min_evidence
        else "insufficient_evidence"
    )
    return {
        "mode": "policy_consistency",
        "ci_fail": False,
        "status": status,
        "meta": {
            "generated_at": generated_at or datetime.now(timezone.utc).isoformat(),
            "records": records_path,
            "policies": list(policies),
            "plan_version": plan_version,
            "rubric_version": rubric_version,
            "task_category": task_category,
            "entrypoint": entrypoint,
            "min_evidence": min_evidence,
        },
        "summary": {
            "records": len(records),
            "usable": len(usable),
            "excluded": len(errors),
            "comparable": total_comparable,
            "agreements": total_agree,
            "has_data": has_data,
            "consistency": (total_agree / total_comparable) if has_data else None,
            "by_policy": {
                k: {kk: vv for kk, vv in v.items() if kk != "mismatches"}
                for k, v in per_policy.items()
            },
        },
        "policy_consistency": per_policy,
        "results": results,
        "errors": errors,
    }


def consistency_from_accuracy_rows(
    rows: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    """Small consistency metric embedded in live routing-accuracy reports.

    Recomputes the saved label policy from result rows and compares it with the
    recorded route. Empty / all-error rows have no data (never a zero score).
    """
    comparable = 0
    agreements = 0
    for row in rows:
        if row.get("status") != "ok":
            continue
        pick = row.get("routed_model")
        expected = row.get("expected_model")
        if not pick or not expected:
            continue
        comparable += 1
        if str(pick) == str(expected):
            agreements += 1
    return {
        "has_data": comparable > 0,
        "comparable": comparable,
        "agreements": agreements,
        "consistency": (agreements / comparable) if comparable else None,
        "policy": "expected_label (offline recompute from saved rows)",
    }
