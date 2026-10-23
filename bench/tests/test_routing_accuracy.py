"""Mocked routing-accuracy + build-routing-eval unit tests."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench.accuracy.runner import run_routing_accuracy
from bench.build_routing_eval import build_corpus, label_prompt, write_corpus
from bench.cli import main
from bench.http_client import ChatResult, EndpointConfig


class TestLabelPrompt(unittest.TestCase):
    def test_explain_large(self) -> None:
        self.assertEqual(label_prompt("Explain photosynthesis in one sentence."), "large")

    def test_systems_mid(self) -> None:
        self.assertEqual(
            label_prompt("What is a common design trade-off when choosing a cache eviction policy?"),
            "mid",
        )

    def test_factoid_small(self) -> None:
        self.assertEqual(label_prompt("What does CPU stand for?"), "small")

    def test_mcq_small(self) -> None:
        self.assertEqual(label_prompt("Which organelle makes ATP?", has_choices=True), "small")


class TestBuildCorpus(unittest.TestCase):
    def test_template_size(self) -> None:
        items = build_corpus()
        self.assertGreaterEqual(len(items), 200)
        intents = {x["intent"] for x in items}
        self.assertIn("large", intents)
        self.assertIn("mid", intents)
        self.assertIn("small", intents)
        self.assertIn("trap", intents)

    def test_write_and_cli(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "eval.json"
            code = main(["build-routing-eval", "--out", str(out)])
            self.assertEqual(code, 0)
            data = json.loads(out.read_text(encoding="utf-8"))
            self.assertGreaterEqual(len(data), 200)


class TestRoutingAccuracy(unittest.TestCase):
    def test_perfect_picks(self) -> None:
        corpus = [
            {
                "id": "a",
                "prompt": "Explain X",
                "expected_model": "deepseek-v4-pro",
            },
            {
                "id": "b",
                "prompt": "What does CPU stand for?",
                "expected_model": "qwen3.5-flash",
            },
            {
                "id": "c",
                "prompt": "trade-off?",
                "expected_model": "glm-5.3",
            },
        ]
        picks = {
            "Explain X": "ariacompute/ariamodel-large",
            "What does CPU stand for?": "ariacompute/ariamodel-small",
            "trade-off?": "ariacompute/ariamodel-mid",
        }

        def fake_chat(cfg, *, model, prompt, max_tokens=16, temperature=0.0, **_k):
            return ChatResult(
                status="ok",
                content="ok",
                model="ignored",
                headers={"x-aria-router-model": picks[prompt]},
                latency_ms=1.0,
            )

        report = run_routing_accuracy(
            corpus=corpus,
            router=EndpointConfig("http://127.0.0.1:8899"),
            entrypoint="ariacompute/semantic-auto",
            chat_fn=fake_chat,
        )
        self.assertEqual(report["summary"]["overall_accuracy"], 1.0)
        self.assertEqual(report["summary"]["correct"], 3)
        self.assertEqual(report["summary"]["errors"], 0)

    def test_wrong_tier_and_missing_header(self) -> None:
        corpus = [
            {"id": "w", "prompt": "Explain Y", "expected_model": "deepseek-v4-pro"},
            {"id": "m", "prompt": "hi", "expected_model": "qwen3.5-flash"},
        ]

        def fake_chat(cfg, *, model, prompt, max_tokens=16, temperature=0.0, **_k):
            if prompt == "Explain Y":
                return ChatResult(
                    status="ok",
                    content="ok",
                    headers={"x-aria-router-model": "ariacompute/ariamodel-small"},
                )
            return ChatResult(status="ok", content="ok", headers={}, model=None)

        report = run_routing_accuracy(
            corpus=corpus,
            router=EndpointConfig("http://r"),
            chat_fn=fake_chat,
        )
        self.assertEqual(report["summary"]["correct"], 0)
        self.assertGreaterEqual(report["summary"]["errors"], 1)
        self.assertLess(report["summary"]["overall_accuracy"], 1.0)

    def test_cli_missing_router(self) -> None:
        code = main(["routing-accuracy", "--report", "/tmp/acc.json"])
        self.assertEqual(code, 2)

    def test_cli_mocked(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            corpus_path = Path(td) / "c.json"
            write_corpus(
                [
                    {
                        "id": "q1",
                        "domain": "chat",
                        "prompt": "hi",
                        "expected_model": "qwen3.5-flash",
                        "intent": "small",
                    }
                ],
                corpus_path,
            )
            report_path = Path(td) / "r.json"

            def fake_chat(cfg, *, model, prompt, max_tokens=16, temperature=0.0, **_k):
                return ChatResult(
                    status="ok",
                    content="ok",
                    headers={"x-aria-router-model": "ariacompute/ariamodel-small"},
                )

            with mock.patch("bench.accuracy.runner.chat_completion", side_effect=fake_chat):
                # CLI imports run_routing_accuracy which defaults chat_fn to chat_completion
                # from the accuracy module's call site — patch where it's used in runner.
                with mock.patch(
                    "bench.cli.run_routing_accuracy",
                    side_effect=lambda **kw: run_routing_accuracy(
                        **{**kw, "chat_fn": fake_chat}
                    ),
                ):
                    code = main(
                        [
                            "routing-accuracy",
                            "--router",
                            "http://127.0.0.1:8899",
                            "--corpus",
                            str(corpus_path),
                            "--report",
                            str(report_path),
                        ]
                    )
            self.assertEqual(code, 0)
            data = json.loads(report_path.read_text(encoding="utf-8"))
            self.assertEqual(data["mode"], "routing_accuracy")
            self.assertEqual(data["summary"]["overall_accuracy"], 1.0)


if __name__ == "__main__":
    unittest.main()
