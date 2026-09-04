"""ADR-040 policies on a routing matrix (pure functions)."""

from __future__ import annotations

from collections import Counter, defaultdict
from typing import Any, Callable, Mapping, Sequence

from .matrix import RoutingMatrix

PickFn = Callable[[str], str | None]  # qid -> model


def always_policy(model: str) -> PickFn:
    def pick(_qid: str) -> str | None:
        return model

    pick.__name__ = f"always_{model}"  # type: ignore[attr-defined]
    return pick


def oracle_quality(matrix: RoutingMatrix) -> PickFn:
    def pick(qid: str) -> str | None:
        qs = matrix.qualities(qid)
        if not qs:
            return None
        # tie-break: prefer lower cost, then name
        costs = matrix.costs(qid)
        best_q = max(qs.values())
        candidates = [m for m, q in qs.items() if q == best_q]
        candidates.sort(key=lambda m: (costs.get(m, 0.0), m))
        return candidates[0]

    return pick


def oracle_cost_optimal(matrix: RoutingMatrix, eps: float = 0.03) -> PickFn:
    """Among models within ``eps`` of best quality, pick lowest cost."""

    def pick(qid: str) -> str | None:
        qs = matrix.qualities(qid)
        if not qs:
            return None
        best_q = max(qs.values())
        costs = matrix.costs(qid)
        eligible = [m for m, q in qs.items() if q >= best_q - eps]
        if not eligible:
            return None
        eligible.sort(key=lambda m: (costs.get(m, float("inf")), -qs[m], m))
        return eligible[0]

    return pick


def router_policy(picks: Mapping[str, str | None]) -> PickFn:
    """Live / precomputed picks per question (must not peek at quality)."""

    def pick(qid: str) -> str | None:
        return picks.get(qid)

    return pick


def _domain_of(qid: str, domains: Mapping[str, str] | None) -> str:
    if domains and qid in domains and domains[qid]:
        return domains[qid]
    # prefix before first '-' if looks like sci-/tech-
    if "-" in qid:
        return qid.split("-", 1)[0]
    return "default"


def domain_router(
    matrix: RoutingMatrix,
    domains: Mapping[str, str] | None = None,
) -> PickFn:
    """Leave-one-out: pick model that wins most often in same domain (excl. self)."""

    # Precompute per-domain win counts (oracle quality winners)
    domain_wins: dict[str, Counter[str]] = defaultdict(Counter)
    for qid in matrix.question_ids:
        oq = oracle_quality(matrix)(qid)
        if oq:
            d = _domain_of(qid, domains)
            domain_wins[d][oq] += 1

    def pick(qid: str) -> str | None:
        d = _domain_of(qid, domains)
        # leave-one-out: subtract this question's oracle winner
        counts = Counter(domain_wins.get(d) or {})
        oq = oracle_quality(matrix)(qid)
        if oq and counts[oq] > 0:
            counts[oq] -= 1
        if not counts or sum(counts.values()) == 0:
            # fallback: global (leave-one-out)
            global_counts: Counter[str] = Counter()
            for q2 in matrix.question_ids:
                if q2 == qid:
                    continue
                w = oracle_quality(matrix)(q2)
                if w:
                    global_counts[w] += 1
            if not global_counts:
                return matrix.models[0] if matrix.models else None
            return global_counts.most_common(1)[0][0]
        return counts.most_common(1)[0][0]

    return pick


def knn_router(matrix: RoutingMatrix, k: int = 3) -> PickFn:
    """Leave-one-out k-NN on question id string overlap (Jaccard of char bigrams).

    Tiny CI heuristic — not a trained embedding k-NN.
    """

    def _bigrams(s: str) -> set[str]:
        s = s.lower()
        if len(s) < 2:
            return {s}
        return {s[i : i + 2] for i in range(len(s) - 1)}

    qids = list(matrix.question_ids)
    feats = {q: _bigrams(q) for q in qids}

    def pick(qid: str) -> str | None:
        if qid not in feats:
            return matrix.models[0] if matrix.models else None
        fa = feats[qid]
        scored: list[tuple[float, str]] = []
        for q2 in qids:
            if q2 == qid:
                continue
            fb = feats[q2]
            if not fa and not fb:
                sim = 1.0
            elif not fa or not fb:
                sim = 0.0
            else:
                sim = len(fa & fb) / max(len(fa | fb), 1)
            scored.append((sim, q2))
        scored.sort(key=lambda t: (-t[0], t[1]))
        neighbors = scored[: max(1, k)]
        votes: Counter[str] = Counter()
        for sim, q2 in neighbors:
            w = oracle_quality(matrix)(q2)
            if w:
                votes[w] += 1 + sim
        if not votes:
            return matrix.models[0] if matrix.models else None
        return votes.most_common(1)[0][0]

    return pick


def evaluate_policy(
    matrix: RoutingMatrix,
    pick_fn: PickFn,
    *,
    policy_name: str,
) -> dict[str, Any]:
    """Mean quality / cost / q_per_dollar for a pick function."""
    qualities: list[float] = []
    costs: list[float] = []
    picks: list[dict[str, Any]] = []
    errors = 0
    for qid in matrix.question_ids:
        model = pick_fn(qid)
        if model is None or matrix.get(qid, model) is None:
            errors += 1
            picks.append({"question_id": qid, "model": model, "status": "error"})
            continue
        cell = matrix.get(qid, model)
        assert cell is not None
        if cell.status != "ok":
            errors += 1
            picks.append(
                {
                    "question_id": qid,
                    "model": model,
                    "status": cell.status,
                    "quality": cell.quality,
                    "cost_usd": cell.cost_usd,
                }
            )
            continue
        qualities.append(cell.quality)
        costs.append(cell.cost_usd)
        picks.append(
            {
                "question_id": qid,
                "model": model,
                "status": "ok",
                "quality": cell.quality,
                "tokens": cell.tokens,
                "cost_usd": cell.cost_usd,
            }
        )
    n = len(qualities)
    mean_q = sum(qualities) / n if n else 0.0
    mean_c = sum(costs) / n if n else 0.0
    qd = (mean_q / mean_c) if mean_c > 0 else (float("inf") if mean_q > 0 else 0.0)
    return {
        "policy": policy_name,
        "mean_quality": mean_q,
        "mean_cost_usd": mean_c,
        "q_per_dollar": qd if qd != float("inf") else None,
        "n": n,
        "errors": errors,
        "picks": picks,
    }


def analyse(
    policy_rows: Sequence[dict[str, Any]],
    *,
    oracle_quality_name: str = "oracle_quality",
    oracle_qd_name: str = "oracle_cost_optimal",
) -> list[dict[str, Any]]:
    """Attach pct_of_oracle_quality and pct_of_oracle_qd to each policy row."""
    by_name = {r["policy"]: r for r in policy_rows}
    oq = by_name.get(oracle_quality_name)
    oqd = by_name.get(oracle_qd_name)
    oq_q = float(oq["mean_quality"]) if oq else 0.0
    # oracle qd: prefer oracle_cost_optimal's q_per_dollar; fallback oracle_quality
    oqd_val = None
    if oqd and oqd.get("q_per_dollar") is not None:
        oqd_val = float(oqd["q_per_dollar"])
    elif oq and oq.get("q_per_dollar") is not None:
        oqd_val = float(oq["q_per_dollar"])

    out: list[dict[str, Any]] = []
    for r in policy_rows:
        row = dict(r)
        mq = float(r.get("mean_quality") or 0.0)
        row["pct_of_oracle_quality"] = (mq / oq_q) if oq_q > 0 else 0.0
        qd = r.get("q_per_dollar")
        if qd is None or oqd_val is None or oqd_val <= 0:
            row["pct_of_oracle_qd"] = 0.0 if qd is None else None
        else:
            row["pct_of_oracle_qd"] = float(qd) / oqd_val
        # drop bulky picks from ladder summary copy? keep them in full row
        out.append(row)
    return out
