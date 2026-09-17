"""Hidden acceptance tests for task gzip. Never shown to workers."""

import gzip
import unittest

from app.config import Config
from app.handlers import reset_items
from app.pipeline import Pipeline, Request, Response
from app.router import build_pipeline

BODY = "0123456789abcdef" * 64  # 1024 bytes


class GzipTest(unittest.TestCase):
    def setUp(self):
        from app.middlewares.gzip import GzipMiddleware

        self.Middleware = GzipMiddleware

    def dispatch(self, accept="gzip, deflate", status=200, body=BODY, encoding=None, min_size=100):
        def handler(request, ctx):
            response = Response.text(body, status)
            response.headers["Vary"] = "Origin"
            if encoding:
                response.headers["Content-Encoding"] = encoding
            return response

        req = Request("GET", "/report")
        if accept is not None:
            req.headers["Accept-Encoding"] = accept
        return Pipeline(handler, [self.Middleware(min_size=min_size)]).dispatch(req)

    def assertUncompressed(self, response):
        self.assertNotIn("Content-Encoding", response.headers)
        self.assertEqual(response.body, BODY.encode())

    def test_exported_and_named(self):
        from app.middlewares import GzipMiddleware

        self.assertIs(GzipMiddleware, self.Middleware)
        self.assertEqual(self.Middleware().name, "gzip")

    def test_compresses(self):
        response = self.dispatch()
        self.assertEqual(response.headers["Content-Encoding"], "gzip")
        self.assertEqual(gzip.decompress(response.body), BODY.encode())
        self.assertEqual(response.headers["Content-Length"], str(len(response.body)))
        self.assertEqual([t.strip() for t in response.headers["Vary"].split(",")], ["Origin", "Accept-Encoding"])

    def test_case_insensitive_token(self):
        self.assertEqual(self.dispatch(accept="deflate, GZIP").headers["Content-Encoding"], "gzip")

    def test_not_accepted(self):
        self.assertUncompressed(self.dispatch(accept=None))
        self.assertUncompressed(self.dispatch(accept="br, deflate"))
        self.assertUncompressed(self.dispatch(accept="gzip;q=0"))

    def test_small_body(self):
        self.assertUncompressed(self.dispatch(min_size=4096))

    def test_already_encoded(self):
        response = self.dispatch(encoding="br")
        self.assertEqual(response.headers["Content-Encoding"], "br")
        self.assertEqual(response.body, BODY.encode())

    def test_no_content_statuses(self):
        self.assertNotIn("Content-Encoding", self.dispatch(status=304).headers)

    def test_build_pipeline(self):
        reset_items()
        req = Request("GET", "/report")
        req.headers["Accept-Encoding"] = "gzip"
        self.assertNotIn("gzip", build_pipeline(Config()).names())
        self.assertNotIn("Content-Encoding", build_pipeline(Config()).dispatch(req).headers)
        pipeline = build_pipeline(Config(gzip_min_size=200))
        self.assertEqual(pipeline.names()[-1], "gzip")
        response = pipeline.dispatch(req)
        self.assertEqual(response.headers["Content-Encoding"], "gzip")
        self.assertTrue(gzip.decompress(response.body).startswith(b"id,name,stock"))


if __name__ == "__main__":
    unittest.main()
