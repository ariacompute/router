"""Policy unit tests on a synthetic routing matrix."""

from __future__ import annotations

import unittest

from bench.routing.matrix import Cell, RoutingMatrix
from bench.routing.policies import (
    always_policy,
    analyse,
    domain_router,
    evaluate_policy,
    knn_router,
    oracle_cost_optimal,
    oracle_quality,
    router_policy,
)


def _matrix() -> RoutingMatrix:
    # q1: large better quality, expensive; small cheap but lower q
    # q2: small best quality and cheap
    cells = {
        ("q1", "local/small"): Cell(quality=0.5, tokens=100, cost_usd=0.01),
        ("q1", "local/large"): Cell(quality=1.0, tokens=200, cost_usd=0.12),
        ("q2", "local/small"): Cell(quality=1.0, tokens=80, cost_usd=0.008),
        ("q2", "local/large"): Cell(quality=0.7, tokens=180, cost_usd=0.11),
        ("sci-a", "local/small"): Cell(quality=0.2, tokens=50, cost_usd=0.005),
        ("sci-a", "local/large"): Cell(quality=0.9, tokens=120, cost_usd=0.07),
        ("sci-b", "local/small"): Cell(quality=0.3, tokens=60, cost_usd=0.006),
        ("sci-b", "local/large"): Cell(quality=1.0, tokens=130, cost_usd=0.08),
        ("tech-a", "local/small"): Cell(quality=1.0, tokens=40, cost_usd=0.004),
        ("tech-a", "local/large"): Cell(quality=0.4, tokens=150, cost_usd=0.09),
    }
    return RoutingMatrix(
        question_ids=["q1", "q2", "sci-a", "sci-b", "tech-a"],
        models=["local/small", "local/large"],
        cells=cells,
    )


class TestPolicies(unittest.TestCase):
    def test_always(self) -> None:
        m = _matrix()
        row = evaluate_policy(m, always_policy("local/small"), policy_name="always_local/small")
        self.assertEqual(row["policy"], "always_local/small")
        self.assertEqual(row["n"], 5)
        self.assertAlmostEqual(row["mean_quality"], (0.5 + 1.0 + 0.2 + 0.3 + 1.0) / 5)

    def test_oracle_quality(self) -> None:
        m = _matrix()
        pick = oracle_quality(m)
        self.assertEqual(pick("q1"), "local/large")
        self.assertEqual(pick("q2"), "local/small")

    def test_oracle_cost_optimal(self) -> None:
        m = _matrix()
        # For q1 best q=1.0 only large; eps won't include small (0.5)
        pick = oracle_cost_optimal(m, eps=0.03)
        self.assertEqual(pick("q1"), "local/large")
        self.assertEqual(pick("q2"), "local/small")
        # Widen eps so small is eligible on q1 if we had closer scores —
        # craft: on sci-a best=0.9 large; small=0.2 still outside eps=0.03
        self.assertEqual(pick("sci-a"), "local/large")

    def test_oracle_cost_optimal_eps_picks_cheaper(self) -> None:
        cells = {
            ("x", "a"): Cell(quality=0.98, tokens=10, cost_usd=0.01),
            ("x", "b"): Cell(quality=1.0, tokens=100, cost_usd=1.0),
        }
        m = RoutingMatrix(question_ids=["x"], models=["a", "b"], cells=cells)
        self.assertEqual(oracle_cost_optimal(m, eps=0.03)("x"), "a")
        self.assertEqual(oracle_cost_optimal(m, eps=0.01)("x"), "b")

    def test_router_policy(self) -> None:
        m = _matrix()
        picks = {"q1": "local/small", "q2": "local/large"}
        # only evaluate those two by subset matrix
        m2 = RoutingMatrix(
            question_ids=["q1", "q2"],
            models=m.models,
            cells={k: v for k, v in m.cells.items() if k[0] in ("q1", "q2")},
        )
        row = evaluate_policy(m2, router_policy(picks), policy_name="aria_router")
        self.assertEqual(row["picks"][0]["model"], "local/small")
        self.assertEqual(row["picks"][1]["model"], "local/large")

    def test_domain_and_knn(self) -> None:
        m = _matrix()
        domains = {
            "sci-a": "sci",
            "sci-b": "sci",
            "tech-a": "tech",
            "q1": "misc",
            "q2": "misc",
        }
        dpick = domain_router(m, domains)
        # sci leave-one-out should favor large
        self.assertEqual(dpick("sci-a"), "local/large")
        kpick = knn_router(m, k=2)
        self.assertIn(kpick("sci-a"), m.models)

    def test_analyse_pct(self) -> None:
        m = _matrix()
        rows = [
            evaluate_policy(m, always_policy("local/small"), policy_name="always_local/small"),
            evaluate_policy(m, oracle_quality(m), policy_name="oracle_quality"),
            evaluate_policy(
                m, oracle_cost_optimal(m, eps=0.03), policy_name="oracle_cost_optimal"
            ),
        ]
        out = analyse(rows)
        oq = next(r for r in out if r["policy"] == "oracle_quality")
        self.assertAlmostEqual(oq["pct_of_oracle_quality"], 1.0)
        always = next(r for r in out if r["policy"].startswith("always_"))
        self.assertLessEqual(always["pct_of_oracle_quality"], 1.0 + 1e-9)
        self.assertIn("pct_of_oracle_qd", always)


if __name__ == "__main__":
    unittest.main()
