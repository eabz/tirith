import io
import json
import unittest

from app.config import Config
from app.handlers import reset_items
from app.pipeline import Request
from app.router import build_pipeline
from app.wsgi import make_wsgi_app, request_from_environ


class RouterTest(unittest.TestCase):
    def setUp(self):
        reset_items()
        self.pipeline = build_pipeline(Config())

    def test_health(self):
        response = self.pipeline.dispatch(Request("GET", "/health"))
        self.assertEqual((response.status, response.body), (200, b"ok"))
        self.assertIn("X-Response-Time", response.headers)
        self.assertEqual(response.headers["X-Content-Type-Options"], "nosniff")

    def test_params_and_404(self):
        self.assertEqual(json.loads(self.pipeline.dispatch(Request("GET", "/items/3")).body)["name"], "crucible")
        self.assertEqual(self.pipeline.dispatch(Request("GET", "/items/99")).status, 404)
        self.assertEqual(self.pipeline.dispatch(Request("GET", "/nope")).status, 404)

    def test_405_lists_allowed(self):
        response = self.pipeline.dispatch(Request("DELETE", "/items"))
        self.assertEqual(response.status, 405)
        self.assertEqual(response.headers["Allow"], "GET, POST")

    def test_create_item(self):
        response = self.pipeline.dispatch(Request("POST", "/items", body=b'{"name": "tongs", "stock": 2}'))
        self.assertEqual(response.status, 201)
        self.assertEqual(json.loads(response.body)["id"], "4")
        self.assertEqual(self.pipeline.dispatch(Request("POST", "/items", body=b"{")).status, 400)

    def test_head_has_no_body(self):
        response = self.pipeline.dispatch(Request("HEAD", "/health"))
        self.assertEqual((response.status, response.body), (200, b""))

    def test_middleware_order(self):
        self.assertEqual(self.pipeline.names(), ["trailing_slash", "timing", "max_body", "security_headers"])

    def test_config_from_env(self):
        config = Config.from_env({"APP_MAX_BODY_BYTES": "10", "APP_DEBUG": "yes", "OTHER": "x"})
        self.assertEqual((config.max_body_bytes, config.debug), (10, True))


class WsgiTest(unittest.TestCase):
    def test_round_trip(self):
        reset_items()
        environ = {
            "REQUEST_METHOD": "GET",
            "PATH_INFO": "/items",
            "QUERY_STRING": "in_stock=1",
            "HTTP_X_CUSTOM": "yes",
            "wsgi.input": io.BytesIO(b""),
        }
        self.assertEqual(request_from_environ(environ).headers["X-Custom"], "yes")
        statuses = []
        body = b"".join(make_wsgi_app(build_pipeline())(environ, lambda s, h: statuses.append(s)))
        self.assertEqual(statuses, ["200 OK"])
        self.assertEqual([row["name"] for row in json.loads(body)["items"]], ["anvil", "crucible"])


if __name__ == "__main__":
    unittest.main()
