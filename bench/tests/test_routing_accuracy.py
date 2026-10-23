"""Mocked routing-accuracy + build-routing-eval unit tests."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench.accuracy.runner import run_routing_accuracy
from bench.build_routing_eval import (
    SMALL,
    MID,
    LARGE,
    build_corpus,
    import_draco,
    import_mmlu,
    label_prompt,
    merge_corpus,
    write_corpus,
)
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

    def test_empty_and_multi_q(self) -> None:
        self.assertEqual(label_prompt(""), "small")
        self.assertEqual(label_prompt("What is REST? What is GraphQL?"), "mid")

    def test_length_heuristics(self) -> None:
        mid_len = "word " * 50  # ~250 chars → mid
        self.assertEqual(label_prompt(mid_len.strip()), "mid")
        large_len = "word " * 90  # ~450 chars → large
        self.assertEqual(label_prompt(large_len.strip()), "large")

    def test_factoid_beats_explain_only_when_no_explain_verb(self) -> None:
        self.assertEqual(label_prompt("What does HTTP stand for?"), "small")
        # explain verb present → large even with stand-for bait
        self.assertEqual(
            label_prompt("Explain nothing — what does DNA stand for?"),
            "large",
        )


class TestBuildCorpus(unittest.TestCase):
    def test_template_size(self) -> None:
        items = build_corpus()
        self.assertGreaterEqual(len(items), 200)
        intents = {x["intent"] for x in items}
        self.assertIn("large", intents)
        self.assertIn("mid", intents)
        self.assertIn("small", intents)
        self.assertIn("trap", intents)
        # Gold labels must use current TokenHub triad upstream ids.
        for it in items:
            self.assertIn(it["expected_model"], {SMALL, MID, LARGE})
            if it["intent"] in ("small", "trap"):
                self.assertEqual(it["expected_model"], SMALL)
            elif it["intent"] == "mid":
                self.assertEqual(it["expected_model"], MID)
            elif it["intent"] == "large":
                self.assertEqual(it["expected_model"], LARGE)

    def test_write_and_cli(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "eval.json"
            code = main(["build-routing-eval", "--out", str(out)])
            self.assertEqual(code, 0)
            data = json.loads(out.read_text(encoding="utf-8"))
            self.assertGreaterEqual(len(data), 200)

    def test_import_mmlu_draco_and_merge(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            mmlu = Path(td) / "mmlu.jsonl"
            draco = Path(td) / "draco.jsonl"
            mmlu.write_text(
                "\n".join(
                    [
                        json.dumps(
                            {
                                "id": "1",
                                "category": "biology",
                                "question": "What does ATP stand for?",
                                "choices": ["A", "B"],
                                "answer": "A",
                            }
                        ),
                        json.dumps({"question": ""}),
                        json.dumps(
                            {
                                "question": "Explain how photosynthesis works in detail.",
                            }
                        ),
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            draco.write_text(
                json.dumps(
                    {
                        "domain": "math",
                        "problem": "x" * 220,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            m_items = import_mmlu(mmlu, limit=10)
            d_items = import_draco(draco, limit=10)
            self.assertEqual(len(m_items), 2)
            self.assertEqual(m_items[0]["expected_model"], SMALL)
            self.assertEqual(m_items[1]["expected_model"], LARGE)
            self.assertEqual(len(d_items), 1)
            self.assertEqual(d_items[0]["expected_model"], MID)
            merged = merge_corpus([m_items, m_items, d_items])
            self.assertEqual(len(merged), 3)

    def test_build_with_optional_imports(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            mmlu = Path(td) / "m.jsonl"
            mmlu.write_text(
                json.dumps({"question": "What does CPU stand for?", "choices": ["A"]}) + "\n",
                encoding="utf-8",
            )
            out = Path(td) / "out.json"
            code = main(
                [
                    "build-routing-eval",
                    "--out",
                    str(out),
                    "--from-mmlu",
                    str(mmlu),
                    "--mmlu-limit",
                    "5",
                ]
            )
            self.assertEqual(code, 0)
            data = json.loads(out.read_text(encoding="utf-8"))
            self.assertTrue(any(str(x["id"]).startswith("mmlu-") for x in data))


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
                "expected_model": "glm-5.3-flash",
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
            {"id": "m", "prompt": "hi", "expected_model": "glm-5.3-flash"},
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

    def test_chat_http_error_counts_as_error(self) -> None:
        corpus = [{"id": "e", "prompt": "hi", "expected_model": SMALL}]

        def fake_chat(cfg, *, model, prompt, max_tokens=16, temperature=0.0, **_k):
            return ChatResult(status="error", error="HTTP 502: upstream", latency_ms=2.0)

        report = run_routing_accuracy(
            corpus=corpus,
            router=EndpointConfig("http://r"),
            chat_fn=fake_chat,
        )
        self.assertEqual(report["summary"]["scored"], 0)
        self.assertEqual(report["summary"]["errors"], 1)
        self.assertEqual(report["summary"]["overall_accuracy"], 0.0)

    def test_custom_pick_map(self) -> None:
        corpus = [{"id": "p", "prompt": "hi", "expected_model": SMALL}]

        def fake_chat(cfg, *, model, prompt, max_tokens=16, temperature=0.0, **_k):
            return ChatResult(
                status="ok",
                content="ok",
                headers={"x-aria-router-model": "vendor/alias-small"},
            )

        report = run_routing_accuracy(
            corpus=corpus,
            router=EndpointConfig("http://r"),
            pick_map={"vendor/alias-small": SMALL},
            chat_fn=fake_chat,
        )
        self.assertEqual(report["summary"]["correct"], 1)
        self.assertEqual(report["results"][0]["routed_model"], SMALL)

    def test_confusion_and_recall(self) -> None:
        corpus = [
            {"id": "ok", "prompt": "a", "expected_model": SMALL},
            {"id": "bad", "prompt": "b", "expected_model": SMALL},
            {"id": "mid", "prompt": "c", "expected_model": MID},
        ]

        def fake_chat(cfg, *, model, prompt, max_tokens=16, temperature=0.0, **_k):
            pick = {
                "a": "ariacompute/ariamodel-small",
                "b": "ariacompute/ariamodel-large",
                "c": "ariacompute/ariamodel-mid",
            }[prompt]
            return ChatResult(
                status="ok",
                content="ok",
                headers={"x-aria-router-model": pick},
            )

        report = run_routing_accuracy(
            corpus=corpus,
            router=EndpointConfig("http://r"),
            chat_fn=fake_chat,
        )
        self.assertEqual(report["summary"]["correct"], 2)
        self.assertAlmostEqual(report["summary"]["overall_accuracy"], 2 / 3)
        self.assertEqual(report["confusion"]["small"]["small"], 1)
        self.assertEqual(report["confusion"]["small"]["large"], 1)
        self.assertEqual(report["summary"]["recall_by_expected_tier"]["small"], 0.5)
        self.assertEqual(report["summary"]["recall_by_expected_tier"]["mid"], 1.0)

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
                        "expected_model": SMALL,
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
