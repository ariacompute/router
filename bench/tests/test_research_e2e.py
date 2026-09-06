"""End-to-end research with fake chat_fn (no network)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from bench.http_client import ChatResult, EndpointConfig
from bench.report import write_reports
from bench.research.runner import load_research_corpus, run_research
from bench.quality.rubric import AXES


class TestResearchE2E(unittest.TestCase):
    def test_label_research_tiny(self) -> None:
        corpus_path = (
            Path(__file__).resolve().parents[1] / "corpus" / "research_tiny.jsonl"
        )
        corpus = load_research_corpus(corpus_path)
        self.assertEqual(len(corpus), 2)

        def chat_fn(cfg: EndpointConfig, *, model: str, prompt: str, **kwargs):
            if model.startswith("ariacompute/"):
                text = (
                    "Vaccines elicit antibody responses and memory cells. "
                    if "vaccine" in prompt.lower()
                    else "CDNs use edge caches near users; origin holds the master copy."
                )
                return ChatResult(
                    status="ok",
                    content=text,
                    completion_tokens=30,
                    model="local/large",
                    headers={"x-aria-router-model": "local/large"},
                )
            # always_large with hits
            if "vaccine" in prompt.lower():
                content = "Adaptive immunity: antibody production and long-lived memory cells."
            else:
                content = "Edge PoPs cache static assets; origin remains authoritative."
            return ChatResult(
                status="ok", content=content, completion_tokens=40, model=model
            )

        report = run_research(
            corpus=corpus,
            pool={"large": EndpointConfig("http://127.0.0.1:9002")},
            model_ids={"large": "local/large"},
            quality="label",
            router=EndpointConfig("http://127.0.0.1:8899"),
            entrypoint="ariacompute/semantic-auto",
            skip_probe=True,
            chat_fn=chat_fn,
        )
        self.assertEqual(report["mode"], "router_research")
        self.assertIs(report["ci_fail"], False)
        systems = {s["system"]: s for s in report["systems"]}
        self.assertIn("always_local/large", systems)
        self.assertIn("aria_router", systems)
        for s in report["systems"]:
            for axis in AXES:
                self.assertIn(axis, s["axes"])
            self.assertIn("delta_vs_best_always", s)

        with tempfile.TemporaryDirectory() as td:
            jp = Path(td) / "research.json"
            write_reports(report, jp)
            md = jp.with_suffix(".md").read_text(encoding="utf-8")
            self.assertIn("Systems", md)

    def test_judge_skip(self) -> None:
        corpus = [
            {
                "id": "t1",
                "domain": "sci",
                "problem": "Say hello",
                "answer": {
                    "sections": [
                        {
                            "name": "factual-accuracy",
                            "weight": 1,
                            "criteria": [{"text": "Greets", "weight": 1}],
                        }
                    ]
                },
                "expected_hits": ["hello"],
            }
        ]

        def chat_fn(cfg, *, model, prompt, **kwargs):
            return ChatResult(status="ok", content="hello world", model=model)

        report = run_research(
            corpus=corpus,
            pool={"small": EndpointConfig("http://s")},
            model_ids={"small": "local/small"},
            quality="judge",
            judge_url=None,
            skip_probe=True,
            chat_fn=chat_fn,
        )
        self.assertTrue(
            any(r.get("status") == "skipped" for r in report["results"])
        )


if __name__ == "__main__":
    unittest.main()
