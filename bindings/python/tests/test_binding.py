import json
import os
import unittest

from aria_router import Router


@unittest.skipUnless(os.environ.get("ARIA_ROUTER_FFI_LIB") and os.environ.get("ARIA_ROUTER_CONFIG"), "need FFI + config")
class BindingTests(unittest.TestCase):
    def setUp(self):
        self.r = Router().init(os.environ["ARIA_ROUTER_CONFIG"])

    def tearDown(self):
        self.r.close()

    def test_init_ok(self):
        m = self.r.models()
        self.assertTrue(any("semantic-auto" in str(x) for x in m.get("data", [])))

    def test_complete_ok(self):
        out = self.r.complete([{"role": "user", "content": "hi"}], {"model": "ariacompute/semantic-auto"})
        self.assertIn("hello-from-router", json.dumps(out))

    def test_last_route(self):
        self.r.complete([{"role": "user", "content": "hi"}], {"model": "ariacompute/semantic-auto"})
        lr = self.r.last_route()
        self.assertEqual(lr.get("layer"), "semantic")

    def test_init_missing_path(self):
        with self.assertRaises(RuntimeError):
            Router().init("/no/such.yaml")

    def test_setup_memory_only(self):
        r = Router()
        r.setup(base_url="http://127.0.0.1:8899", token="t")
        self.assertEqual(r.setup_status()["token"], "t")
        r.setup_clear()
        self.assertEqual(r.setup_status()["token"], "")


if __name__ == "__main__":
    unittest.main()
