"""Hidden acceptance tests for task ctx-interface. Never shown to workers."""

import inspect
import unittest

from app.config import Config
from app.middlewares import MaxBodyMiddleware, SecurityHeadersMiddleware, TimingMiddleware, TrailingSlashMiddleware
from app.pipeline import Context, Middleware, Pipeline, Request, Response
from app.router import build_pipeline


class FakeClock:
    def __init__(self, now=100.0):
        self.now = now

    def __call__(self):
        return self.now


class Spy(Middleware):
    name = "spy"

    def __init__(self, seen, short=False):
        self.seen = seen
        self.short = short

    def process_request(self, request, ctx):
        self.seen.append(("req", ctx))
        return Response.text("short", 418) if self.short else None

    def process_response(self, request, response, ctx):
        self.seen.append(("resp", ctx))
        return response


class CtxInterfaceTest(unittest.TestCase):
    def test_signatures(self):
        self.assertEqual(list(inspect.signature(Middleware.process_request).parameters), ["self", "request", "ctx"])
        self.assertEqual(
            list(inspect.signature(Middleware.process_response).parameters), ["self", "request", "response", "ctx"]
        )

    def test_same_context_everywhere(self):
        seen = []

        def handler(request, ctx):
            seen.append(("handler", ctx))
            return Response.text("ok")

        response = Pipeline(handler, [Spy(seen), Spy(seen)]).dispatch(Request("GET", "/"))
        self.assertEqual(response.status, 200)
        self.assertEqual(len(seen), 5)
        first = seen[0][1]
        self.assertIsInstance(first, Context)
        for _, ctx in seen:
            self.assertIs(ctx, first)

    def test_new_context_per_request(self):
        seen = []
        pipeline = Pipeline(lambda r, c: Response.text("ok"), [Spy(seen)])
        pipeline.dispatch(Request("GET", "/"))
        pipeline.dispatch(Request("GET", "/"))
        self.assertIsNot(seen[0][1], seen[2][1])

    def test_short_circuit_passes_context_outwards(self):
        seen = []
        response = Pipeline(lambda r, c: Response.text("ok"), [Spy(seen), Spy(seen, short=True)]).dispatch(
            Request("GET", "/")
        )
        self.assertEqual(response.status, 418)
        self.assertEqual([kind for kind, _ in seen], ["req", "req", "resp", "resp"])

    def test_request_extras_removed(self):
        self.assertFalse(hasattr(Request("GET", "/"), "extras"))

    def test_existing_middlewares_migrated(self):
        clock = FakeClock()

        def slow(request, ctx):
            clock.now += 0.25
            return Response.text("ok")

        timed = Pipeline(slow, [TimingMiddleware(clock=clock)]).dispatch(Request("GET", "/"))
        self.assertEqual(timed.headers["X-Response-Time"], "250.0ms")
        ok = lambda r, c: Response.text("ok")  # noqa: E731
        self.assertEqual(Pipeline(ok, [MaxBodyMiddleware(3)]).dispatch(Request("POST", "/", body=b"long")).status, 413)
        self.assertEqual(Pipeline(ok, [TrailingSlashMiddleware()]).dispatch(Request("GET", "/items/")).status, 308)
        secured = Pipeline(ok, [SecurityHeadersMiddleware()]).dispatch(Request("GET", "/"))
        self.assertEqual(secured.headers["X-Content-Type-Options"], "nosniff")

    def test_build_pipeline_still_works(self):
        response = build_pipeline(Config()).dispatch(Request("GET", "/health"))
        self.assertEqual((response.status, response.body), (200, b"ok"))
        self.assertIn("X-Response-Time", response.headers)


if __name__ == "__main__":
    unittest.main()
