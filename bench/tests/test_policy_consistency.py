"""Offline policy consistency: normal paths + evidence/exception paths."""

from __future__ import annotations

import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench.cli import build_parser, main
from bench.consistency import (
    POLICIES,
    consistency_from_accuracy_rows,
    load_decision_records,
    recommend,
    run_policy_consistency,
    tier_of,
)
from bench.report import render_markdown

SAMPLE = [
    {
        "id": "q1",
        "decision": {"model": "m/small", "confidence": 0.9},
        "eligible": ["m/small", "m/large"],
        "expected_model": "m/small",
        "plan_version": "plan-1",
        "rubric_version": "rubric-1",
        "task_category": "draft",
        "entrypoint": "ep",
    },
    {
        "id": "q2",
        "decision": {"model": "m/large", "confidence": 0.2},
        "eligible": ["m/small", "m/large"],
        "expected_model": "m/large",
        "plan_version": "plan-1",
        "rubric_version": "rubric-1",
        "task_category": "draft",
        "entrypoint": "ep",
    },
]


class TestPolicyConsistency(unittest.TestCase):
    def test_recommend_is_deterministic_and_within_pool(self) -> None:
        self.assertEqual(tier_of("ariacompute/ariamodel-small"), "small")
        self.assertEqual(tier_of("deepseek-v4-pro"), "large")
        self.assertEqual(tier_of(None), "unknown")
        self.assertEqual(recommend(SAMPLE[0], "cheapest")[0], "m/small")
        self.assertEqual(recommend(SAMPLE[0], "strongest")[0], "m/large")
        self.assertEqual(recommend(SAMPLE[0], "expected_label")[0], "m/small")
        self.assertEqual(recommend(SAMPLE[0], "recorded_confidence")[0], "m/small")
        self.assertEqual(recommend(SAMPLE[1], "recorded_confidence")[0], "m/small")
        self.assertEqual(
            recommend({"decision": {"model": "x"}}, "expected_label"),
            (None, "no matching saved label in eligible pool"),
        )
        self.assertEqual(recommend({}, "cheapest"), (None, "no_eligible_models"))
        with self.assertRaises(ValueError):
            recommend(SAMPLE[0], "bogus")
        self.assertIn("recorded_confidence", POLICIES)

    def test_run_consistency_agrees_and_reports_mismatches(self) -> None:
        rep = run_policy_consistency(
            records=SAMPLE,
            policies=["cheapest", "expected_label"],
            plan_version="plan-1",
            rubric_version="rubric-1",
            task_category="draft",
            entrypoint="ep",
            generated_at="2026-01-01T00:00:00+00:00",
        )
        self.assertEqual(rep["mode"], "policy_consistency")
        self.assertFalse(rep["ci_fail"])
        self.assertEqual(rep["status"], "ready")
        self.assertEqual(rep["summary"]["usable"], 2)
        self.assertEqual(rep["summary"]["comparable"], 4)
        self.assertEqual(rep["summary"]["agreements"], 3)
        self.assertAlmostEqual(rep["summary"]["consistency"], 0.75)
        cheapest = rep["policy_consistency"]["cheapest"]
        self.assertEqual(cheapest["agreements"], 1)
        self.assertEqual(len(cheapest["mismatches"]), 1)
        self.assertEqual(cheapest["mismatches"][0]["recorded"], "m/large")
        md = render_markdown(rep)
        self.assertIn("Policy consistency (offline recompute)", md)
        self.assertIn("plan-1", md)

    def test_evidence_gate_excludes_mismatched_versions_and_entrypoints(self) -> None:
        rep = run_policy_consistency(
            records=SAMPLE
            + [
                {**SAMPLE[0], "id": "wrong-plan", "plan_version": "plan-2"},
                {**SAMPLE[0], "id": "wrong-rubric", "rubric_version": "rubric-2"},
                {**SAMPLE[0], "id": "wrong-category", "task_category": "review"},
                {**SAMPLE[0], "id": "wrong-ep", "entrypoint": "other"},
                {
                    "id": "no-decision",
                    "plan_version": "plan-1",
                    "rubric_version": "rubric-1",
                    "task_category": "draft",
                    "entrypoint": "ep",
                },
            ],
            plan_version="plan-1",
            rubric_version="rubric-1",
            task_category="draft",
            entrypoint="ep",
            min_evidence=3,
        )
        reasons = {e["reason"] for e in rep["errors"]}
        self.assertEqual(
            reasons,
            {
                "plan_version_mismatch",
                "rubric_version_mismatch",
                "task_category_mismatch",
                "entrypoint_mismatch",
                "missing_recorded_decision",
            },
        )
        self.assertEqual(rep["status"], "insufficient_evidence")
        self.assertTrue(rep["summary"]["has_data"])

    def test_no_data_is_not_zero(self) -> None:
        rep = run_policy_consistency(records=[])
        self.assertIsNone(rep["summary"]["consistency"])
        self.assertFalse(rep["summary"]["has_data"])
        self.assertEqual(rep["status"], "insufficient_evidence")
        blank = run_policy_consistency(records=[{"decision": {"model": "m"}}], policies=["expected_label"])
        self.assertIsNone(blank["summary"]["consistency"])
        self.assertFalse(blank["summary"]["has_data"])
        with self.assertRaises(ValueError):
            run_policy_consistency(records=[], policies=["bogus"])
        with self.assertRaises(ValueError):
            run_policy_consistency(records=[], min_evidence=-1)

    def test_load_decision_records_json_jsonl_and_errors(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            jl = root / "r.jsonl"
            jl.write_text(
                "\n".join(json.dumps(r) for r in SAMPLE) + "\n\n", encoding="utf-8"
            )
            self.assertEqual(len(load_decision_records(jl)), 2)
            js = root / "r.json"
            js.write_text(json.dumps({"records": SAMPLE}), encoding="utf-8")
            self.assertEqual(len(load_decision_records(js)), 2)
            js.write_text(json.dumps(SAMPLE[0]), encoding="utf-8")
            self.assertEqual(len(load_decision_records(js)), 1)
            js.write_text("42", encoding="utf-8")
            with self.assertRaises(ValueError):
                load_decision_records(js)
            jl.write_text("not json\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                load_decision_records(jl)
            jl.write_text("[1,2]\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                load_decision_records(jl)
            with self.assertRaises(FileNotFoundError):
                load_decision_records(root / "missing.jsonl")

    def test_cli_policy_consistency_and_bad_cli(self) -> None:
        ns = build_parser().parse_args(
            ["policy-consistency", "--records", "r.jsonl", "--policy", "cheapest"]
        )
        self.assertEqual(ns.cmd, "policy-consistency")
        self.assertEqual(ns.policy, ["cheapest"])
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            src = root / "r.jsonl"
            src.write_text("\n".join(json.dumps(r) for r in SAMPLE), encoding="utf-8")
            out = root / "out.json"
            buf = io.StringIO()
            with mock.patch("sys.stdout", buf):
                code = main(
                    [
                        "policy-consistency",
                        "--records",
                        str(src),
                        "--report",
                        str(out),
                        "--plan-version",
                        "plan-1",
                    ]
                )
            self.assertEqual(code, 0)
            self.assertTrue(out.exists())
            self.assertIn('"mode": "policy_consistency"', out.read_text(encoding="utf-8"))
            self.assertIn('"ci_fail": false', buf.getvalue())
            self.assertTrue((root / "out.md").exists())
            with mock.patch("sys.stdout", buf):
                missing = main(
                    ["policy-consistency", "--records", str(root / "absent.jsonl")]
                )
            self.assertEqual(missing, 2)
        with self.assertRaises(SystemExit) as ctx:
            build_parser().parse_args(
                ["policy-consistency", "--records", "x", "--policy", "bogus"]
            )
        self.assertEqual(ctx.exception.code, 2)

    def test_accuracy_row_metric(self) -> None:
        rows = [
            {"status": "ok", "routed_model": "m/small", "expected_model": "m/small"},
            {"status": "ok", "routed_model": "m/large", "expected_model": "m/small"},
            {"status": "error", "routed_model": "m/small", "expected_model": "m/small"},
            {"status": "ok", "routed_model": "m/small", "expected_model": None},
        ]
        out = consistency_from_accuracy_rows(rows)
        self.assertTrue(out["has_data"])
        self.assertEqual(out["comparable"], 2)
        self.assertEqual(out["agreements"], 1)
        self.assertAlmostEqual(out["consistency"], 0.5)
        empty = consistency_from_accuracy_rows([{"status": "error"}])
        self.assertFalse(empty["has_data"])
        self.assertIsNone(empty["consistency"])


if __name__ == "__main__":
    unittest.main()
