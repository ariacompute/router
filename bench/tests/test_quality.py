"""label / overlap / judge parsing tests."""

from __future__ import annotations

import unittest

from bench.http_client import ChatResult, EndpointConfig
from bench.quality.judge import judge_overall, parse_score_0_1
from bench.quality.label import keyword_hit_score, label_score
from bench.quality.overlap import token_overlap


class TestQuality(unittest.TestCase):
    def test_label_score(self) -> None:
        self.assertEqual(label_score("local/small", "local/small"), 1.0)
        self.assertEqual(label_score("local/large", "local/small"), 0.0)
        self.assertEqual(label_score(None, "local/small"), 0.0)
        self.assertEqual(label_score("x", None), 0.0)

    def test_keyword_hits(self) -> None:
        self.assertAlmostEqual(
            keyword_hit_score("has antibody and memory cells", ["antibody", "memory"]),
            1.0,
        )
        self.assertAlmostEqual(
            keyword_hit_score("only antibody here", ["antibody", "memory"]),
            0.5,
        )

    def test_token_overlap_jaccard(self) -> None:
        self.assertEqual(token_overlap("a b c", "a b c"), 1.0)
        self.assertEqual(token_overlap("", ""), 1.0)
        self.assertEqual(token_overlap("a", ""), 0.0)
        # {a,b} vs {b,c} → 1/3
        self.assertAlmostEqual(token_overlap("a b", "b c"), 1.0 / 3.0)

    def test_parse_score(self) -> None:
        self.assertEqual(parse_score_0_1("0.75"), 0.75)
        self.assertEqual(parse_score_0_1("Score: 1"), 1.0)
        self.assertEqual(parse_score_0_1("nope"), None)

    def test_judge_skip_without_url(self) -> None:
        out = judge_overall(prompt="q", completion="a", judge_url=None)
        self.assertEqual(out["status"], "skipped")
        self.assertIsNone(out["score"])

    def test_judge_with_fake_chat(self) -> None:
        def fake(cfg, **kwargs):
            return ChatResult(status="ok", content="0.42")

        out = judge_overall(
            prompt="q",
            completion="a",
            judge_url="http://judge.test",
            chat_fn=fake,
        )
        self.assertEqual(out["status"], "ok")
        self.assertAlmostEqual(out["score"], 0.42)


if __name__ == "__main__":
    unittest.main()
