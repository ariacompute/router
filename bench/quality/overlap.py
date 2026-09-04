"""Mode A overlap: whitespace-token Jaccard (engine bench metrics.token_overlap)."""

from __future__ import annotations


def token_overlap(a: str, b: str) -> float:
    """Whitespace-token Jaccard; same semantics as engine gen_compare / metrics."""
    ta = a.split()
    tb = b.split()
    if not ta and not tb:
        return 1.0
    if not ta or not tb:
        return 0.0
    sa, sb = set(ta), set(tb)
    return len(sa & sb) / max(len(sa | sb), 1)
