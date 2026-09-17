"""Hidden acceptance tests for task access-log. Never shown to workers."""

import json
import logging
import unittest

from app.config import Config
from app.pipeline import Middleware, Pipeline, Request, Response
from app.router import build_pipeline

KEYS = {"method", "path", "status", "duration_ms", "request_id", "user", "client_ip"}


class FakeClock:
    def __init__(self, now=50.0):
        self.now = now

    def __call__(self):
        return self.now


class Identify(Middleware):
    name = "identify"

    def __init__(self, stop=False):
        self.stop = stop

    def process_request(self, request, ctx):
        ctx.request_id = "r-1"
        ctx.user = "bob"
        return Response.text("unauthorized", 401) if self.stop else None


class Collect(logging.Handler):
    def __init__(self):
        super().__init__()
        self.records = []

    def emit(self, record):
        self.records.append(record)


class AccessLogTest(unittest.TestCase):
    def setUp(self):
        from app.middlewares.access_log import AccessLogMiddleware

        self.Middleware = AccessLogMiddleware
        self.clock = FakeClock()

    def slow(self, request, ctx):
        self.clock.now += 0.01234
        return Response.text("ok")

    def lines(self, cm):
        return [json.loads(record.getMessage()) for record in cm.records]

    def test_exported_and_named(self):
        from app.middlewares import AccessLogMiddleware

        self.assertIs(AccessLogMiddleware, self.Middleware)
        self.assertEqual(self.Middleware().name, "access_log")

    def test_line_fields(self):
        logger = logging.getLogger("hidden.access")
        pipeline = Pipeline(self.slow, [self.Middleware(logger=logger, clock=self.clock)])
        with self.assertLogs("hidden.access", level="INFO") as cm:
            pipeline.dispatch(Request("GET", "/items", client_ip="10.0.0.9"))
        self.assertEqual(len(cm.records), 1)
        self.assertEqual(cm.records[0].levelno, logging.INFO)
        line = self.lines(cm)[0]
        self.assertEqual(set(line), KEYS)
        self.assertEqual(
            line,
            {
                "client_ip": "10.0.0.9",
                "duration_ms": 12.3,
                "method": "GET",
                "path": "/items",
                "request_id": None,
                "status": 200,
                "user": None,
            },
        )
        self.assertEqual(cm.records[0].getMessage(), json.dumps(line, sort_keys=True))

    def test_default_logger_and_context_fields(self):
        pipeline = Pipeline(self.slow, [self.Middleware(clock=self.clock), Identify()])
        with self.assertLogs("app.access", level="INFO") as cm:
            pipeline.dispatch(Request("POST", "/items"))
        line = self.lines(cm)[0]
        self.assertEqual((line["request_id"], line["user"], line["method"]), ("r-1", "bob", "POST"))

    def test_logs_short_circuited_responses(self):
        pipeline = Pipeline(self.slow, [self.Middleware(clock=self.clock), Identify(stop=True)])
        with self.assertLogs("app.access", level="INFO") as cm:
            pipeline.dispatch(Request("GET", "/items"))
        self.assertEqual(self.lines(cm)[0]["status"], 401)

    def test_build_pipeline(self):
        collect = Collect()
        logger = logging.getLogger("app.access")
        logger.addHandler(collect)
        old_level = logger.level
        logger.setLevel(logging.INFO)
        try:
            build_pipeline(Config(clock=self.clock)).dispatch(Request("GET", "/health"))
            self.assertEqual(collect.records, [])
            pipeline = build_pipeline(Config(access_log=True, clock=self.clock))
            self.assertIn("access_log", pipeline.names())
            pipeline.dispatch(Request("GET", "/health"))
            self.assertEqual(len(collect.records), 1)
            self.assertEqual(json.loads(collect.records[0].getMessage())["status"], 200)
        finally:
            logger.removeHandler(collect)
            logger.setLevel(old_level)


if __name__ == "__main__":
    unittest.main()
