"""Multi-router parse + dual live-router e2e."""

from __future__ import annotations

import unittest

from bench.http_client import ChatResult, EndpointConfig
from bench.router_targets import parse_router_args, resolve_pick
from bench.routing.runner import run_routing
from bench.research.runner import run_research


class TestRouterTargets(unittest.TestCase):
    def test_bare_url_maps_aria(self) -> None:
        specs = parse_router_args(["http://127.0.0.1:8899"])
        self.assertEqual(len(specs), 1)
        self.assertEqual(specs[0].name, "aria_router")
        self.assertEqual(specs[0].pick_headers, ["x-aria-router-model"])

    def test_named_routers_and_entrypoints(self) -> None:
        specs = parse_router_args(
            [
                "aria_router=http://127.0.0.1:8899",
                "vllm_sr=http://127.0.0.1:8890",
            ],
            entrypoint_args=[
                "aria_router=aria/semantic-auto",
                "vllm_sr=auto",
            ],
            pick_header_args=["aria_router=x-aria-router-model"],
        )
        by = {s.name: s for s in specs}
        self.assertEqual(by["aria_router"].entrypoint, "aria/semantic-auto")
        self.assertEqual(by["vllm_sr"].entrypoint, "auto")
        self.assertEqual(by["vllm_sr"].pick_headers, [])

    def test_resolve_pick_map(self) -> None:
        mid, err = resolve_pick(
            "Qwen/Qwen3-0.6B",
            models=["local/small"],
            alias_to_model={"small": "local/small"},
            pick_map={"Qwen/Qwen3-0.6B": "local/small"},
        )
        self.assertEqual(mid, "local/small")
        self.assertIsNone(err)


class TestDualRouterE2E(unittest.TestCase):
    def test_routing_two_routers(self) -> None:
        corpus = [
            {"id": "sci-q1", "prompt": "explain photosynthesis", "expected_model": "local/large"},
            {"id": "tech-q1", "prompt": "what is a reverse proxy", "expected_model": "local/large"},
            {"id": "gen-q1", "prompt": "say hi", "expected_model": "local/small"},
        ]

        def chat_fn(cfg: EndpointConfig, *, model: str, prompt: str, **kwargs):
            if "8899" in cfg.base_url:
                pick = "local/large" if "photosynthesis" in prompt or "proxy" in prompt else "local/small"
                return ChatResult(
                    status="ok",
                    content="aria",
                    completion_tokens=5,
                    model=pick,
                    headers={"x-aria-router-model": pick},
                )
            if "8890" in cfg.base_url:
                # VSR: body model only; always pick large
                return ChatResult(
                    status="ok",
                    content="vsr",
                    completion_tokens=5,
                    model="local/large",
                    headers={},
                )
            return ChatResult(
                status="ok",
                content=f"from {model}",
                completion_tokens=10,
                model=model,
            )

        from bench.router_targets import RouterSpec

        routers = [
            RouterSpec(
                "aria_router",
                EndpointConfig("http://127.0.0.1:8899"),
                entrypoint="aria/semantic-auto",
                pick_headers=["x-aria-router-model"],
            ),
            RouterSpec(
                "vllm_sr",
                EndpointConfig("http://127.0.0.1:8890"),
                entrypoint="auto",
                pick_headers=[],
            ),
        ]
        report = run_routing(
            corpus=corpus,
            pool={
                "small": EndpointConfig("http://127.0.0.1:9001"),
                "large": EndpointConfig("http://127.0.0.1:9002"),
            },
            model_ids={"small": "local/small", "large": "local/large"},
            quality="label",
            routers=routers,
            skip_probe=True,
            chat_fn=chat_fn,
            include_domain_knn=False,
        )
        policies = {r["policy"] for r in report["ladder"]}
        self.assertIn("aria_router", policies)
        self.assertIn("vllm_sr", policies)

    def test_research_two_routers(self) -> None:
        corpus = [
            {
                "id": "t1",
                "domain": "sci",
                "problem": "vaccine memory cells antibody",
                "answer": {
                    "sections": [
                        {
                            "name": "factual-accuracy",
                            "weight": 1,
                            "criteria": [{"text": "mentions antibody", "weight": 1}],
                        }
                    ]
                },
                "expected_hits": ["antibody", "memory"],
            }
        ]

        def chat_fn(cfg, *, model, prompt, **kwargs):
            return ChatResult(
                status="ok",
                content="antibody and memory cells",
                completion_tokens=8,
                model="local/large",
                headers={"x-aria-router-model": "local/large"},
            )

        from bench.router_targets import RouterSpec

        report = run_research(
            corpus=corpus,
            pool={"large": EndpointConfig("http://l")},
            model_ids={"large": "local/large"},
            quality="label",
            routers=[
                RouterSpec(
                    "aria_router",
                    EndpointConfig("http://a"),
                    pick_headers=["x-aria-router-model"],
                ),
                RouterSpec("vllm_sr", EndpointConfig("http://v"), entrypoint="auto", pick_headers=[]),
            ],
            skip_probe=True,
            chat_fn=chat_fn,
        )
        systems = {s["system"] for s in report["systems"]}
        self.assertIn("aria_router", systems)
        self.assertIn("vllm_sr", systems)


if __name__ == "__main__":
    unittest.main()
