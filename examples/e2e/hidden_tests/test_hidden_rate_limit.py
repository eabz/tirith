"""Hidden acceptance tests for task rate-limit. Never shown to workers."""

import unittest

from app.config import Config
from app.pipeline import Pipeline, Request, Response
from app.router import build_pipeline


class FakeClock:
    def __init__(self, now=1000.0):
        self.now = now

    def __call__(self):
        return self.now


def ok(request, ctx):
    return Response.text("ok")


class RateLimitTest(unittest.TestCase):
    def setUp(self):
        from app.middlewares.ratelimit import RateLimitMiddleware

        self.Middleware = RateLimitMiddleware
        self.clock = FakeClock()

    def pipeline(self, rate, burst):
        return Pipeline(ok, [self.Middleware(rate, burst, clock=self.clock)])

    def test_exported_and_named(self):
        from app.middlewares import RateLimitMiddleware

        self.assertIs(RateLimitMiddleware, self.Middleware)
        self.assertEqual(self.Middleware(1.0, 1).name, "rate_limit")

    def test_burst_then_429(self):
        pipeline = self.pipeline(1.0, 2)
        first = pipeline.dispatch(Request("GET", "/"))
        second = pipeline.dispatch(Request("GET", "/"))
        third = pipeline.dispatch(Request("GET", "/"))
        self.assertEqual([first.status, second.status, third.status], [200, 200, 429])
        self.assertEqual(first.headers["X-RateLimit-Remaining"], "1")
        self.assertEqual(second.headers["X-RateLimit-Remaining"], "0")
        self.assertEqual(third.body, b"rate limit exceeded")
        self.assertEqual(third.headers["Retry-After"], "1")

    def test_refill(self):
        pipeline = self.pipeline(1.0, 1)
        self.assertEqual(pipeline.dispatch(Request("GET", "/")).status, 200)
        self.assertEqual(pipeline.dispatch(Request("GET", "/")).status, 429)
        self.clock.now += 1.0
        self.assertEqual(pipeline.dispatch(Request("GET", "/")).status, 200)

    def test_refill_is_capped_at_burst(self):
        pipeline = self.pipeline(1.0, 3)
        pipeline.dispatch(Request("GET", "/"))
        self.clock.now += 100.0
        self.assertEqual(pipeline.dispatch(Request("GET", "/")).headers["X-RateLimit-Remaining"], "2")

    def test_retry_after_rounds_up(self):
        pipeline = self.pipeline(0.25, 1)
        pipeline.dispatch(Request("GET", "/"))
        self.assertEqual(pipeline.dispatch(Request("GET", "/")).headers["Retry-After"], "4")

    def test_buckets_are_per_client_ip(self):
        pipeline = self.pipeline(1.0, 1)
        self.assertEqual(pipeline.dispatch(Request("GET", "/", client_ip="10.0.0.1")).status, 200)
        self.assertEqual(pipeline.dispatch(Request("GET", "/", client_ip="10.0.0.1")).status, 429)
        self.assertEqual(pipeline.dispatch(Request("GET", "/", client_ip="10.0.0.2")).status, 200)

    def test_build_pipeline(self):
        self.assertNotIn("rate_limit", build_pipeline(Config()).names())
        pipeline = build_pipeline(Config(rate_limit_per_second=1.0, rate_limit_burst=1, clock=self.clock))
        self.assertIn("rate_limit", pipeline.names())
        self.assertEqual(pipeline.dispatch(Request("GET", "/health")).status, 200)
        self.assertEqual(pipeline.dispatch(Request("GET", "/health")).status, 429)
        self.clock.now += 2.0
        self.assertEqual(pipeline.dispatch(Request("GET", "/health")).status, 200)


if __name__ == "__main__":
    unittest.main()
