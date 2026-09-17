"""Hidden acceptance tests for task correlation-id (a near-duplicate of
request-id). Only the observable behavior of build_pipeline is tested, so
the request-id implementation satisfies it. Never shown to workers."""

import re
import unittest

from app.config import Config
from app.pipeline import Request, Response
from app.router import Router, build_pipeline

HEX32 = re.compile(r"[0-9a-f]{32}")


class CorrelationIdTest(unittest.TestCase):
    def setUp(self):
        self.seen = []
        self.router = Router()
        self.router.add("GET", "/probe", self.probe)
        self.router.add("POST", "/probe", self.probe)

    def probe(self, request, ctx):
        self.seen.append(ctx.request_id)
        return Response.text("ok")

    def test_minted_when_absent(self):
        response = build_pipeline(Config(), router=self.router).dispatch(Request("GET", "/probe"))
        self.assertRegex(response.headers["X-Request-ID"], HEX32)
        self.assertEqual(self.seen, [response.headers["X-Request-ID"]])

    def test_client_id_honored(self):
        req = Request("GET", "/probe")
        req.headers["X-Request-ID"] = "support-ticket_42.b"
        response = build_pipeline(Config(), router=self.router).dispatch(req)
        self.assertEqual(response.headers["X-Request-ID"], "support-ticket_42.b")
        self.assertEqual(self.seen, ["support-ticket_42.b"])

    def test_insane_id_replaced(self):
        req = Request("GET", "/probe")
        req.headers["X-Request-ID"] = "<script>alert(1)</script>"
        response = build_pipeline(Config(), router=self.router).dispatch(req)
        self.assertRegex(response.headers["X-Request-ID"], HEX32)

    def test_errors_and_short_circuits_carry_id(self):
        pipeline = build_pipeline(Config(max_body_bytes=4), router=self.router)
        self.assertRegex(pipeline.dispatch(Request("GET", "/missing")).headers["X-Request-ID"], HEX32)
        too_big = pipeline.dispatch(Request("POST", "/probe", body=b"0123456789"))
        self.assertEqual(too_big.status, 413)
        self.assertRegex(too_big.headers["X-Request-ID"], HEX32)


if __name__ == "__main__":
    unittest.main()
