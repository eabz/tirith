import unittest

from app.pipeline import Context, Headers, HttpError, Middleware, Pipeline, Request, Response


class Recorder(Middleware):
    def __init__(self, name, log, short_circuit=False):
        self.name = name
        self.log = log
        self.short_circuit = short_circuit

    def process_request(self, request, ctx):
        self.log.append("req:" + self.name)
        ctx.values.setdefault("seen", []).append(self.name)
        if self.short_circuit:
            return Response.text("stopped by " + self.name, 418)
        return None

    def process_response(self, request, response, ctx):
        self.log.append("resp:" + self.name)
        return response


def ok_handler(request, ctx):
    return Response.text("handled")


class HeadersTest(unittest.TestCase):
    def test_case_insensitive(self):
        headers = Headers({"Content-Type": "text/plain"})
        self.assertEqual(headers["content-type"], "text/plain")
        self.assertIn("CONTENT-TYPE", headers)
        headers["content-type"] = "application/json"
        self.assertEqual(headers.items(), [("content-type", "application/json")])

    def test_add_vary_deduplicates(self):
        headers = Headers()
        headers.add_vary("Origin")
        headers.add_vary("origin")
        headers.add_vary("Accept-Encoding")
        self.assertEqual(headers["Vary"], "Origin, Accept-Encoding")


class PipelineTest(unittest.TestCase):
    def test_order(self):
        log = []
        pipeline = Pipeline(ok_handler, [Recorder("a", log), Recorder("b", log)])
        response = pipeline.dispatch(Request("GET", "/"))
        self.assertEqual(response.body, b"handled")
        self.assertEqual(log, ["req:a", "req:b", "resp:b", "resp:a"])

    def test_short_circuit_skips_inner_middlewares_and_handler(self):
        log = []
        called = []
        pipeline = Pipeline(
            lambda r, c: called.append(1) or Response.text("x"),
            [Recorder("a", log), Recorder("b", log, short_circuit=True), Recorder("c", log)],
        )
        response = pipeline.dispatch(Request("GET", "/"))
        self.assertEqual(response.status, 418)
        self.assertEqual(called, [])
        self.assertEqual(log, ["req:a", "req:b", "resp:b", "resp:a"])

    def test_context_values_are_shared_between_middlewares(self):
        log = []
        seen = []
        Pipeline(lambda r, c: seen.append(c.values["seen"]) or Response.text("ok"), [Recorder("a", log), Recorder("b", log)]).dispatch(Request("GET", "/"))
        self.assertEqual(seen, [["a", "b"]])

    def test_handler_receives_context(self):
        seen = []

        def handler(request, ctx):
            seen.append(ctx)
            return Response.text("ok")

        Pipeline(handler, clock=lambda: 42.0).dispatch(Request("GET", "/"))
        self.assertIsInstance(seen[0], Context)
        self.assertEqual(seen[0].started_at, 42.0)

    def test_http_error_and_crash(self):
        def missing(request, ctx):
            raise HttpError(404, "gone")

        def crash(request, ctx):
            raise RuntimeError("boom")

        self.assertEqual(Pipeline(missing).dispatch(Request("GET", "/")).status, 404)
        with self.assertLogs("app.pipeline", level="ERROR"):
            self.assertEqual(Pipeline(crash).dispatch(Request("GET", "/")).status, 500)

    def test_json_response_sets_length(self):
        response = Response.json({"b": 1, "a": 2})
        self.assertEqual(response.body, b'{"a": 2, "b": 1}')
        self.assertEqual(response.headers["Content-Length"], str(len(response.body)))


if __name__ == "__main__":
    unittest.main()
