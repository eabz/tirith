"""Hidden acceptance tests for task request-id. Never shown to workers."""

import re
import unittest

from app.config import Config
from app.pipeline import Middleware, Pipeline, Request, Response
from app.router import build_pipeline

HEX32 = re.compile(r"[0-9a-f]{32}")


class Stop(Middleware):
    name = "stop"

    def process_request(self, request, ctx):
        return Response.text("nope", 403)


class RequestIdTest(unittest.TestCase):
    def setUp(self):
        from app.middlewares.request_id import RequestIdMiddleware

        self.Middleware = RequestIdMiddleware
        self.seen = []

    def handler(self, request, ctx):
        self.seen.append(ctx.request_id)
        return Response.text("ok")

    def dispatch(self, middleware, incoming=None, header="X-Request-ID"):
        req = Request("GET", "/")
        if incoming is not None:
            req.headers[header] = incoming
        return Pipeline(self.handler, [middleware]).dispatch(req)

    def test_exported_and_named(self):
        from app.middlewares import RequestIdMiddleware

        self.assertIs(RequestIdMiddleware, self.Middleware)
        self.assertEqual(self.Middleware().name, "request_id")

    def test_generates_hex_id(self):
        response = self.dispatch(self.Middleware())
        self.assertRegex(response.headers["X-Request-ID"], HEX32)
        self.assertEqual(self.seen, [response.headers["X-Request-ID"]])

    def test_reuses_valid_incoming(self):
        response = self.dispatch(self.Middleware(), incoming="abc-123.X_y")
        self.assertEqual(response.headers["X-Request-ID"], "abc-123.X_y")
        self.assertEqual(self.seen, ["abc-123.X_y"])

    def test_replaces_invalid_incoming(self):
        for bad in ("bad id!", "x" * 65, ""):
            response = self.dispatch(self.Middleware(), incoming=bad)
            self.assertRegex(response.headers["X-Request-ID"], HEX32)

    def test_64_chars_is_valid(self):
        self.assertEqual(self.dispatch(self.Middleware(), incoming="a" * 64).headers["X-Request-ID"], "a" * 64)

    def test_custom_generator_and_header(self):
        response = self.dispatch(self.Middleware(header="X-Trace-Id", generator=lambda: "fixed"))
        self.assertEqual(response.headers["X-Trace-Id"], "fixed")
        self.assertEqual(self.seen, ["fixed"])

    def test_short_circuited_response_carries_id(self):
        response = Pipeline(self.handler, [self.Middleware(), Stop()]).dispatch(Request("GET", "/"))
        self.assertEqual(response.status, 403)
        self.assertRegex(response.headers["X-Request-ID"], HEX32)

    def test_build_pipeline_outermost(self):
        pipeline = build_pipeline(Config())
        self.assertEqual(pipeline.names()[0], "request_id")
        self.assertRegex(pipeline.dispatch(Request("GET", "/health")).headers["X-Request-ID"], HEX32)

    def test_build_pipeline_header_name(self):
        req = Request("GET", "/health")
        req.headers["X-Trace-Id"] = "trace-7"
        response = build_pipeline(Config(request_id_header="X-Trace-Id")).dispatch(req)
        self.assertEqual(response.headers["X-Trace-Id"], "trace-7")


if __name__ == "__main__":
    unittest.main()
