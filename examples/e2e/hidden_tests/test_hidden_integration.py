"""Hidden cross-task tests: middleware ordering in build_pipeline, the file
every task edits. Scored separately from per-task acceptance. Never shown
to workers."""

import gzip
import json
import unittest

from app.config import Config
from app.handlers import reset_items
from app.pipeline import Request
from app.router import build_pipeline

ORIGIN = "https://console.example"


class FakeClock:
    def __init__(self, now=10.0):
        self.now = now

    def __call__(self):
        return self.now


def everything(**overrides):
    values = dict(
        cors_allowed_origins=(ORIGIN,),
        api_tokens={"t1": "alice"},
        rate_limit_per_second=100.0,
        rate_limit_burst=100,
        access_log=True,
        gzip_min_size=200,
    )
    values.update(overrides)
    return Config(**values)


class IntegrationTest(unittest.TestCase):
    def setUp(self):
        reset_items()

    def test_cors_preflight_needs_no_token(self):
        req = Request("OPTIONS", "/items")
        req.headers["Origin"] = ORIGIN
        req.headers["Access-Control-Request-Method"] = "POST"
        self.assertEqual(build_pipeline(everything()).dispatch(req).status, 204)

    def test_rate_limit_applies_before_auth(self):
        pipeline = build_pipeline(everything(rate_limit_per_second=1.0, rate_limit_burst=1, clock=FakeClock()))
        with self.assertLogs("app.access", level="INFO"):
            self.assertEqual(pipeline.dispatch(Request("GET", "/items")).status, 401)
            self.assertEqual(pipeline.dispatch(Request("GET", "/items")).status, 429)

    def test_rejections_carry_request_id_and_are_logged(self):
        pipeline = build_pipeline(everything())
        with self.assertLogs("app.access", level="INFO") as cm:
            response = pipeline.dispatch(Request("GET", "/items"))
        self.assertEqual(response.status, 401)
        line = json.loads(cm.records[-1].getMessage())
        self.assertEqual(line["status"], 401)
        self.assertEqual(line["request_id"], response.headers["X-Request-ID"])

    def test_authenticated_user_is_logged(self):
        req = Request("GET", "/items")
        req.headers["Authorization"] = "Bearer t1"
        with self.assertLogs("app.access", level="INFO") as cm:
            self.assertEqual(build_pipeline(everything()).dispatch(req).status, 200)
        self.assertEqual(json.loads(cm.records[-1].getMessage())["user"], "alice")

    def test_everything_on(self):
        req = Request("GET", "/report")
        req.headers["Authorization"] = "Bearer t1"
        req.headers["Origin"] = ORIGIN
        req.headers["Accept-Encoding"] = "gzip"
        with self.assertLogs("app.access", level="INFO"):
            response = build_pipeline(everything()).dispatch(req)
        self.assertEqual(response.status, 200)
        self.assertEqual(response.headers["Access-Control-Allow-Origin"], ORIGIN)
        self.assertIn("X-Request-ID", response.headers)
        self.assertIn("X-Response-Time", response.headers)
        self.assertIn("X-RateLimit-Remaining", response.headers)
        self.assertTrue(gzip.decompress(response.body).startswith(b"id,name,stock"))


if __name__ == "__main__":
    unittest.main()
