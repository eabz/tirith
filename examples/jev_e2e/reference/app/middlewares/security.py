"""Conservative security headers on every response."""

from typing import Optional

from app.pipeline import Context, Middleware, Request, Response

DEFAULTS = (
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
)


class SecurityHeadersMiddleware(Middleware):
    """Sets headers a handler did not set itself; never overwrites."""

    name = "security_headers"

    def __init__(self, frame_options: Optional[str] = "DENY"):
        self._frame_options = frame_options

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        for name, value in DEFAULTS:
            if name not in response.headers:
                response.headers[name] = value
        if self._frame_options and "X-Frame-Options" not in response.headers:
            response.headers["X-Frame-Options"] = self._frame_options
        return response
