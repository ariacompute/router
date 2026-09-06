"""End-to-end routing with fake chat_fn (no network)."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from bench.http_client import ChatResult, EndpointConfig
from bench.report import write_reports
from bench.routing.runner import load_routing_corpus, run_routing


class TestRoutingE2E(unittest.TestCase):
    def test_label_routing_tiny(self) -> None:
        corpus_path = (
            Path(__file__).resolve().parents[1] / "corpus" / "routing_tiny.json"
        )
        corpus = load_routing_corpus(corpus_path)

        def chat_fn(cfg: EndpointConfig, *, model: str, prompt: str, **kwargs):
            # Router path
            if "8899" in cfg.base_url or model.startswith("ariacompute/"):
                # Pick expected model based on prompt keywords for sci-q1 / tech-q1 → large
                if "photosynthesis" in prompt or "reverse proxy" in prompt:
                    pick = "local/large"
                else:
                    pick = "local/small"
                return ChatResult(
                    status="ok",
                    content=f"routed answer for {pick}",
                    completion_tokens=10,
                    model=pick,
                    headers={"x-aria-router-model": pick},
                )
            # Pool path
            return ChatResult(
                status="ok",
                content=f"answer from {model}: {prompt[:20]}",
                completion_tokens=20 if "large" in model else 8,
                model=model,
            )

        pool = {
            "small": EndpointConfig("http://127.0.0.1:9001"),
            "large": EndpointConfig("http://127.0.0.1:9002"),
        }
        router = EndpointConfig("http://127.0.0.1:8899")
        report = run_routing(
            corpus=corpus,
            pool=pool,
            model_ids={"small": "local/small", "large": "local/large"},
            quality="label",
            router=router,
            entrypoint="ariacompute/semantic-auto",
            skip_probe=True,
            chat_fn=chat_fn,
        )
        self.assertEqual(report["mode"], "router_routing")
        self.assertIs(report["ci_fail"], False)
        policies = {r["policy"] for r in report["ladder"]}
        self.assertIn("oracle_quality", policies)
        self.assertTrue(any(p.startswith("always_") for p in policies))
        self.assertIn("aria_router", policies)
        oq = next(r for r in report["ladder"] if r["policy"] == "oracle_quality")
        self.assertAlmostEqual(oq["pct_of_oracle_quality"], 1.0)
        self.assertGreater(oq["mean_quality"], 0.0)

        with tempfile.TemporaryDirectory() as td:
            jp = Path(td) / "r.json"
            write_reports(report, jp)
            self.assertTrue(jp.exists())
            self.assertTrue(jp.with_suffix(".md").exists())
            data = json.loads(jp.read_text(encoding="utf-8"))
            self.assertEqual(data["mode"], "router_routing")

    def test_judge_skips_without_url(self) -> None:
        corpus = [
            {"id": "q", "prompt": "hi", "expected_model": "local/small"},
        ]

        def chat_fn(cfg, *, model, prompt, **kwargs):
            return ChatResult(status="ok", content="hello", completion_tokens=2, model=model)

        report = run_routing(
            corpus=corpus,
            pool={"small": EndpointConfig("http://s")},
            model_ids={"small": "local/small"},
            quality="judge",
            judge_url=None,
            skip_probe=True,
            chat_fn=chat_fn,
            include_domain_knn=False,
        )
        # cells skipped
        statuses = {c["status"] for c in report["matrix"]["cells"]}
        self.assertIn("skipped", statuses)


if __name__ == "__main__":
    unittest.main()
