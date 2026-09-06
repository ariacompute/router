"""CLI help / parse / download-draco skip tests."""

from __future__ import annotations

import io
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench.cli import build_parser, main


class TestCli(unittest.TestCase):
    def test_help_routing(self) -> None:
        p = build_parser()
        ns = p.parse_args(
            [
                "routing",
                "--pool",
                "small=http://127.0.0.1:9",
                "--quality",
                "label",
            ]
        )
        self.assertEqual(ns.cmd, "routing")
        self.assertEqual(ns.quality, "label")
        self.assertTrue(ns.corpus.endswith("routing_tiny.json"))

    def test_research_defaults(self) -> None:
        p = build_parser()
        ns = p.parse_args(["research", "--pool", "large=http://x"])
        self.assertEqual(ns.cmd, "research")
        self.assertTrue(ns.corpus.endswith("research_tiny.jsonl"))

    def test_list_corpus(self) -> None:
        code = main(["list-corpus"])
        self.assertEqual(code, 0)

    def test_list_corpus_json(self) -> None:
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            code = main(["list-corpus", "--json"])
        self.assertEqual(code, 0)
        self.assertIn("routing_tiny.json", buf.getvalue())

    def test_missing_pool_exit_2(self) -> None:
        code = main(["routing", "--report", "/tmp/nope.json"])
        self.assertEqual(code, 2)

    def test_bare_router_url_compat(self) -> None:
        p = build_parser()
        ns = p.parse_args(
            [
                "routing",
                "--pool",
                "small=http://127.0.0.1:9",
                "--router",
                "http://127.0.0.1:8899",
            ]
        )
        self.assertEqual(ns.router, ["http://127.0.0.1:8899"])

    def test_compare_defaults(self) -> None:
        p = build_parser()
        ns = p.parse_args(["compare", "--pool", "base=http://x"])
        self.assertEqual(ns.cmd, "compare")
        self.assertTrue(ns.corpus.endswith("mmlu_tiny.jsonl"))

    def test_download_mmlu_skip(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "m.jsonl"

            def boom(*_a, **_k):
                raise OSError("network down")

            with mock.patch("urllib.request.urlopen", side_effect=boom):
                code = main(["download-mmlu", "--out", str(out)])
            self.assertEqual(code, 0)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
