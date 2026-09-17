"""Cross-origin resource sharing."""

from typing import Iterable, Optional

from app.pipeline import Context, Middleware, Request, Response


class CorsMiddleware(Middleware):
    name = "cors"

    def __init__(
        self,
        allowed_origins: Iterable[str],
        allow_methods: Iterable[str] = ("GET", "POST", "PUT", "PATCH", "DELETE"),
        allow_headers: Iterable[str] = ("Authorization", "Content-Type"),
        max_age: int = 600,
    ):
        self._origins = tuple(allowed_origins)
        self._methods = ", ".join(allow_methods)
        self._headers = ", ".join(allow_headers)
        self._max_age = max_age

    def _allowed(self, origin: str) -> bool:
        return "*" in self._origins or origin in self._origins

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        origin = request.header("Origin")
        if not origin or request.method != "OPTIONS" or request.header("Access-Control-Request-Method") is None:
            return None
        if not self._allowed(origin):
            return Response.text("origin not allowed", 403)
        response = Response.empty(204)
        response.headers["Access-Control-Allow-Origin"] = origin
        response.headers["Access-Control-Allow-Methods"] = self._methods
        response.headers["Access-Control-Allow-Headers"] = self._headers
        response.headers["Access-Control-Max-Age"] = str(self._max_age)
        response.headers.add_vary("Origin")
        ctx.values["cors.preflight"] = True
        return response

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        origin = request.header("Origin")
        if not origin or ctx.values.get("cors.preflight") or not self._allowed(origin):
            return response
        response.headers["Access-Control-Allow-Origin"] = origin
        response.headers.add_vary("Origin")
        return response
