"""MCQ grade + compare e2e (mock HTTP)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from bench.compare.grade import extract_letter, format_mcq_prompt, grade_answer
from bench.compare.runner import load_compare_corpus, run_compare
from bench.http_client import ChatResult, EndpointConfig
from bench.report import write_reports
from bench.router_targets import RouterSpec


class TestGrade(unittest.TestCase):
    def test_extract_letter(self) -> None:
        self.assertEqual(extract_letter("The answer is B."), "B")
        self.assertEqual(extract_letter("I choose (C)"), "C")

    def test_grade_mcq(self) -> None:
        g = grade_answer(completion="Answer: B", gold="B")
        self.assertTrue(g["is_correct"])
        g2 = grade_answer(completion="Answer: A", gold="B")
        self.assertFalse(g2["is_correct"])

    def test_grade_yes_no(self) -> None:
        self.assertTrue(grade_answer(completion="Yes, it is.", gold="yes")["is_correct"])

    def test_format_prompt(self) -> None:
        p = format_mcq_prompt("Q?", ["One", "Two"])
        self.assertIn("A. One", p)
        self.assertIn("B. Two", p)


class TestCompareE2E(unittest.TestCase):
    def test_mmlu_tiny(self) -> None:
        corpus_path = Path(__file__).resolve().parents[1] / "corpus" / "mmlu_tiny.jsonl"
        corpus = load_compare_corpus(corpus_path)
        self.assertGreaterEqual(len(corpus), 8)

        answers = {str(item["id"]): str(item["answer"]) for item in corpus}

        def chat_fn(cfg: EndpointConfig, *, model: str, prompt: str, **kwargs):
            # Find which item by matching question snippet
            gold = "B"
            for item in corpus:
                if item["question"][:20] in prompt:
                    gold = str(item["answer"])
                    break
            if "8899" in cfg.base_url:
                # aria: correct letter
                return ChatResult(
                    status="ok",
                    content=f"Answer: {gold}",
                    completion_tokens=3,
                    prompt_tokens=20,
                    model="local/base",
                    latency_ms=12.0,
                    headers={"x-aria-router-model": "local/base"},
                )
            if "8890" in cfg.base_url:
                # vsr: always wrong letter A (unless gold is A)
                wrong = "A" if gold.upper() != "A" else "B"
                # for yes/no gold
                if gold.lower() in ("yes", "no"):
                    wrong = "no" if gold.lower() == "yes" else "yes"
                    content = wrong
                elif len(gold) == 1:
                    content = f"Answer: {wrong}"
                else:
                    content = "wrong"
                return ChatResult(
                    status="ok",
                    content=content,
                    completion_tokens=4,
                    prompt_tokens=20,
                    model="local/base",
                    latency_ms=40.0,
                )
            # always_base: correct
            if gold.lower() in ("yes", "no"):
                content = gold.lower()
            elif len(gold) == 1:
                content = f"The answer is {gold}."
            else:
                content = gold
            return ChatResult(
                status="ok",
                content=content,
                completion_tokens=5,
                prompt_tokens=18,
                model=model,
                latency_ms=10.0,
            )

        report = run_compare(
            corpus=corpus,
            pool={"base": EndpointConfig("http://127.0.0.1:8000")},
            model_ids={"base": "local/base"},
            routers=[
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
            ],
            skip_probe=True,
            chat_fn=chat_fn,
        )
        self.assertEqual(report["mode"], "router_compare")
        by = {s["system"]: s for s in report["systems"]}
        self.assertIn("always_local/base", by)
        self.assertIn("aria_router", by)
        self.assertIn("vllm_sr", by)
        self.assertGreater(by["aria_router"]["accuracy"], by["vllm_sr"]["accuracy"])
        self.assertIn("latency_ms", by["aria_router"])
        self.assertIn("avg_completion_tokens", by["aria_router"])

        with tempfile.TemporaryDirectory() as td:
            jp = Path(td) / "c.json"
            write_reports(report, jp)
            md = jp.with_suffix(".md").read_text(encoding="utf-8")
            self.assertIn("accuracy", md.lower())
            self.assertIn("Systems (accuracy", md)

        # silence unused
        self.assertTrue(answers)


if __name__ == "__main__":
    unittest.main()
