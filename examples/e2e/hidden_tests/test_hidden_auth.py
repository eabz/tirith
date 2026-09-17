"""Hidden acceptance tests for task auth. Never shown to workers."""

import unittest

from app.config import Config
from app.pipeline import Pipeline, Request, Response
from app.router import build_pipeline

PLAIN = 'Bearer realm="app"'
INVALID = 'Bearer realm="app", error="invalid_token"'


class AuthTest(unittest.TestCase):
    def setUp(self):
        from app.middlewares.auth import BearerAuthMiddleware

        self.Middleware = BearerAuthMiddleware
        self.users = []

    def handler(self, request, ctx):
        self.users.append(ctx.user)
        return Response.text("ok")

    def dispatch(self, path="/items", authorization=None, **kwargs):
        req = Request("GET", path)
        if authorization is not None:
            req.headers["Authorization"] = authorization
        return Pipeline(self.handler, [self.Middleware({"t1": "alice"}, **kwargs)]).dispatch(req)

    def test_exported_and_named(self):
        from app.middlewares import BearerAuthMiddleware

        self.assertIs(BearerAuthMiddleware, self.Middleware)
        self.assertEqual(self.Middleware({}).name, "auth")

    def test_missing_token(self):
        response = self.dispatch()
        self.assertEqual((response.status, response.body), (401, b"unauthorized"))
        self.assertEqual(response.headers["WWW-Authenticate"], PLAIN)
        self.assertEqual(self.users, [])

    def test_non_bearer_scheme(self):
        response = self.dispatch(authorization="Basic dDE6")
        self.assertEqual(response.status, 401)
        self.assertEqual(response.headers["WWW-Authenticate"], PLAIN)

    def test_unknown_token(self):
        response = self.dispatch(authorization="Bearer nope")
        self.assertEqual(response.status, 401)
        self.assertEqual(response.headers["WWW-Authenticate"], INVALID)

    def test_valid_token_sets_user(self):
        self.assertEqual(self.dispatch(authorization="Bearer t1").status, 200)
        self.assertEqual(self.dispatch(authorization="bearer t1").status, 200)
        self.assertEqual(self.users, ["alice", "alice"])

    def test_public_paths(self):
        self.assertEqual(self.dispatch(path="/health").status, 200)
        self.assertEqual(self.dispatch(path="/public/status").status, 200)
        self.assertEqual(self.dispatch(path="/healthz").status, 401)
        self.assertEqual(self.dispatch(path="/publicity").status, 401)

    def test_custom_public_paths(self):
        self.assertEqual(self.dispatch(path="/open", public_paths=("/open",)).status, 200)
        self.assertEqual(self.dispatch(path="/health", public_paths=("/open",)).status, 401)

    def test_build_pipeline(self):
        self.assertNotIn("auth", build_pipeline(Config()).names())
        pipeline = build_pipeline(Config(api_tokens={"t1": "alice"}))
        self.assertIn("auth", pipeline.names())
        self.assertEqual(pipeline.dispatch(Request("GET", "/items")).status, 401)
        self.assertEqual(pipeline.dispatch(Request("GET", "/health")).status, 200)
        req = Request("GET", "/items")
        req.headers["Authorization"] = "Bearer t1"
        self.assertEqual(pipeline.dispatch(req).status, 200)


if __name__ == "__main__":
    unittest.main()
