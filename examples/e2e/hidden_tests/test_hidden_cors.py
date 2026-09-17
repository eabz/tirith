"""Hidden acceptance tests for task cors. Never shown to workers."""

import unittest

from app.config import Config
from app.pipeline import Pipeline, Request, Response
from app.router import build_pipeline

ORIGIN = "https://console.example"


def request(method="GET", path="/items", origin=None, preflight=None):
    req = Request(method, path)
    if origin:
        req.headers["Origin"] = origin
    if preflight:
        req.headers["Access-Control-Request-Method"] = preflight
    return req


class CorsTest(unittest.TestCase):
    def setUp(self):
        from app.middlewares.cors import CorsMiddleware

        self.Middleware = CorsMiddleware
        self.calls = []

    def handler(self, req, ctx):
        self.calls.append(req.method)
        response = Response.text("ok")
        response.headers["Vary"] = "Accept-Encoding"
        return response

    def pipeline(self, origins=(ORIGIN,)):
        return Pipeline(self.handler, [self.Middleware(origins)])

    def test_exported_and_named(self):
        from app.middlewares import CorsMiddleware

        self.assertIs(CorsMiddleware, self.Middleware)
        self.assertEqual(self.Middleware([ORIGIN]).name, "cors")

    def test_preflight_allowed(self):
        response = self.pipeline().dispatch(request("OPTIONS", origin=ORIGIN, preflight="POST"))
        self.assertEqual(response.status, 204)
        self.assertEqual(self.calls, [])
        self.assertEqual(response.headers["Access-Control-Allow-Origin"], ORIGIN)
        self.assertEqual(response.headers["Access-Control-Allow-Methods"], "GET, POST, PUT, PATCH, DELETE")
        self.assertEqual(response.headers["Access-Control-Allow-Headers"], "Authorization, Content-Type")
        self.assertEqual(response.headers["Access-Control-Max-Age"], "600")
        self.assertIn("Origin", [t.strip() for t in response.headers["Vary"].split(",")])

    def test_preflight_disallowed(self):
        response = self.pipeline().dispatch(request("OPTIONS", origin="https://evil.example", preflight="POST"))
        self.assertEqual(response.status, 403)
        self.assertEqual([h for h in response.headers if h.lower().startswith("access-control-")], [])

    def test_options_without_request_method_is_not_preflight(self):
        response = self.pipeline().dispatch(request("OPTIONS", origin=ORIGIN))
        self.assertEqual(self.calls, ["OPTIONS"])
        self.assertEqual(response.status, 200)

    def test_actual_request_allowed_keeps_vary(self):
        response = self.pipeline().dispatch(request(origin=ORIGIN))
        self.assertEqual(response.status, 200)
        self.assertEqual(response.headers["Access-Control-Allow-Origin"], ORIGIN)
        self.assertEqual([t.strip() for t in response.headers["Vary"].split(",")], ["Accept-Encoding", "Origin"])

    def test_actual_request_disallowed(self):
        response = self.pipeline().dispatch(request(origin="https://evil.example"))
        self.assertEqual(response.status, 200)
        self.assertNotIn("Access-Control-Allow-Origin", response.headers)

    def test_no_origin_untouched(self):
        response = self.pipeline().dispatch(request())
        self.assertNotIn("Access-Control-Allow-Origin", response.headers)
        self.assertEqual(response.headers["Vary"], "Accept-Encoding")

    def test_wildcard_echoes_origin(self):
        response = self.pipeline(("*",)).dispatch(request(origin="https://any.example"))
        self.assertEqual(response.headers["Access-Control-Allow-Origin"], "https://any.example")

    def test_build_pipeline(self):
        self.assertNotIn("cors", build_pipeline(Config()).names())
        pipeline = build_pipeline(Config(cors_allowed_origins=(ORIGIN,)))
        self.assertIn("cors", pipeline.names())
        response = pipeline.dispatch(request("OPTIONS", origin=ORIGIN, preflight="GET"))
        self.assertEqual(response.status, 204)


if __name__ == "__main__":
    unittest.main()
