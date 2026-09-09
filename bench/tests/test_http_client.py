"""http_client retry behavior (no live network)."""

from __future__ import annotations

import unittest
from unittest import mock

from bench.http_client import EndpointConfig, _is_transient_error, chat_completion


class TestHttpClient(unittest.TestCase):
    def test_transient_markers(self) -> None:
        self.assertTrue(_is_transient_error("[SSL: UNEXPECTED_EOF_WHILE_READING] EOF"))
        self.assertTrue(_is_transient_error("timed out"))
        self.assertFalse(_is_transient_error("HTTP 401: unauthorized"))

    def test_retries_then_succeeds(self) -> None:
        cfg = EndpointConfig("http://example.test", timeout_s=5)
        ok_body = (
            b'{"id":"1","choices":[{"message":{"role":"assistant","content":"hi"}}],'
            b'"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2},"model":"m"}'
        )
        calls = {"n": 0}

        class FakeResp:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def read(self):
                return ok_body

            def getcode(self):
                return 200

            @property
            def headers(self):
                return {}

        def fake_urlopen(req, timeout=None):
            calls["n"] += 1
            if calls["n"] == 1:
                raise OSError("[SSL: UNEXPECTED_EOF_WHILE_READING] EOF occurred")
            return FakeResp()

        with mock.patch("bench.http_client.urllib.request.urlopen", side_effect=fake_urlopen):
            with mock.patch("bench.http_client.time.sleep", return_value=None):
                result = chat_completion(cfg, model="m", prompt="hi", retries=5)
        self.assertEqual(result.status, "ok")
        self.assertEqual(result.content, "hi")
        self.assertEqual(calls["n"], 2)


if __name__ == "__main__":
    unittest.main()
