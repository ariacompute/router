"""Rubric parse + aggregate tests."""

from __future__ import annotations

import json
import unittest

from bench.quality.rubric import aggregate_rubric, parse_rubric, score_label_rubric


class TestRubric(unittest.TestCase):
    def test_parse_sections(self) -> None:
        raw = {
            "sections": [
                {
                    "name": "factual-accuracy",
                    "weight": 0.5,
                    "criteria": [{"text": "A", "weight": 1}],
                },
                {
                    "name": "presentation",
                    "weight": 0.5,
                    "criteria": ["Clear writing"],
                },
            ]
        }
        r = parse_rubric(raw)
        self.assertEqual(len(r["sections"]), 2)
        self.assertAlmostEqual(sum(s["weight"] for s in r["sections"]), 1.0)

    def test_parse_json_string(self) -> None:
        r = parse_rubric(json.dumps({"factual-accuracy": {"weight": 1, "criteria": ["x"]}}))
        self.assertEqual(r["sections"][0]["axis"], "factual-accuracy")

    def test_aggregate(self) -> None:
        rubric = parse_rubric(
            {
                "sections": [
                    {
                        "name": "factual-accuracy",
                        "weight": 0.5,
                        "criteria": [
                            {"text": "c1", "weight": 1},
                            {"text": "c2", "weight": 1},
                        ],
                    },
                    {
                        "name": "citation",
                        "weight": 0.5,
                        "criteria": [{"text": "c3", "weight": 1}],
                    },
                ]
            }
        )
        agg = aggregate_rubric(
            rubric["sections"],
            [[True, False], [True]],
        )
        # section0 = 0.5, section1 = 1.0 → overall 0.75
        self.assertAlmostEqual(agg["score"], 0.75)
        self.assertIn("factual-accuracy", agg["axes"])
        self.assertIn("citation", agg["axes"])

    def test_label_expected_hits(self) -> None:
        rubric = parse_rubric(
            {
                "sections": [
                    {
                        "name": "factual-accuracy",
                        "weight": 1.0,
                        "criteria": [{"text": "Mentions antibodies", "weight": 1}],
                    }
                ]
            }
        )
        agg = score_label_rubric(
            "The antibody response matters",
            rubric,
            expected_hits=["antibody", "memory"],
        )
        self.assertAlmostEqual(agg["score"], 0.5)
        self.assertEqual(agg["label_mode"], "expected_hits")


if __name__ == "__main__":
    unittest.main()
