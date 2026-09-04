"""Mode A label: expected_model match → 1.0 / 0.0."""

from __future__ import annotations


def label_score(picked_model: str | None, expected_model: str | None) -> float:
    """Decision correctness for routing cells / research decision proxy."""
    if not expected_model:
        return 0.0
    if not picked_model:
        return 0.0
    return 1.0 if picked_model.strip() == expected_model.strip() else 0.0


def keyword_hit_score(text: str, expected_hits: list[str] | None) -> float:
    """Fraction of expected_hits keywords found in text (case-insensitive)."""
    if not expected_hits:
        return 0.0
    lower = (text or "").lower()
    hits = sum(1 for h in expected_hits if h and h.lower() in lower)
    return hits / len(expected_hits)
