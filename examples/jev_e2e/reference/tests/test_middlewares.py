import unittest

from app.middlewares import (
    MaxBodyMiddleware,
    SecurityHeadersMiddleware,
    TimingMiddleware,
    TrailingSlashMiddleware,
)
from app.pipeline import Pipeline, Request, Response


def ok_handler(request, ctx):
    return Response.text("ok")


class FakeClock:
    def __init__(self, now=100.0):
        self.now = now

    def __call__(self):
        return self.now


class TimingTest(unittest.TestCase):
    def test_header(self):
        clock = FakeClock()

        def slow(request, ctx):
            clock.now += 0.25
            return Response.text("ok")

        response = Pipeline(slow, [TimingMiddleware(clock=clock)]).dispatch(Request("GET", "/"))
        self.assertEqual(response.headers["X-Response-Time"], "250.0ms")


class SecurityHeadersTest(unittest.TestCase):
    def test_defaults_do_not_overwrite(self):
        def framed(request, ctx):
            response = Response.text("ok")
            response.headers["X-Frame-Options"] = "SAMEORIGIN"
            return response

        response = Pipeline(framed, [SecurityHeadersMiddleware()]).dispatch(Request("GET", "/"))
        self.assertEqual(response.headers["X-Frame-Options"], "SAMEORIGIN")
        self.assertEqual(response.headers["X-Content-Type-Options"], "nosniff")


class MaxBodyTest(unittest.TestCase):
    def setUp(self):
        self.pipeline = Pipeline(ok_handler, [MaxBodyMiddleware(10)])

    def test_body_over_limit(self):
        self.assertEqual(self.pipeline.dispatch(Request("POST", "/", body=b"x" * 11)).status, 413)

    def test_declared_length_over_limit(self):
        request = Request("POST", "/")
        request.headers["Content-Length"] = "5000"
        self.assertEqual(self.pipeline.dispatch(request).status, 413)

    def test_invalid_length(self):
        request = Request("POST", "/")
        request.headers["Content-Length"] = "lots"
        self.assertEqual(self.pipeline.dispatch(request).status, 400)

    def test_within_limit(self):
        self.assertEqual(self.pipeline.dispatch(Request("POST", "/", body=b"hello")).status, 200)


class TrailingSlashTest(unittest.TestCase):
    def test_redirect_keeps_query(self):
        pipeline = Pipeline(ok_handler, [TrailingSlashMiddleware()])
        response = pipeline.dispatch(Request("GET", "/items/", query={"in_stock": "1"}))
        self.assertEqual(response.status, 308)
        self.assertEqual(response.headers["Location"], "/items?in_stock=1")

    def test_root_and_post_untouched(self):
        pipeline = Pipeline(ok_handler, [TrailingSlashMiddleware()])
        self.assertEqual(pipeline.dispatch(Request("GET", "/")).status, 200)
        self.assertEqual(pipeline.dispatch(Request("POST", "/items/")).status, 200)


if __name__ == "__main__":
    unittest.main()
