"""Gzip response compression."""

import gzip as _gzip
from typing import Optional

from app.pipeline import Context, Middleware, Request, Response


def _accepts_gzip(header: Optional[str]) -> bool:
    for part in (header or "").split(","):
        token, _, params = part.strip().partition(";")
        if token.strip().lower() != "gzip":
            continue
        q = params.strip().replace(" ", "").lower()
        return q not in ("q=0", "q=0.0", "q=0.00", "q=0.000")
    return False


class GzipMiddleware(Middleware):
    name = "gzip"

    def __init__(self, min_size: int = 500, level: int = 6):
        self._min_size = min_size
        self._level = level

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        if (
            response.status in (204, 304)
            or len(response.body) < self._min_size
            or "Content-Encoding" in response.headers
            or not _accepts_gzip(request.header("Accept-Encoding"))
        ):
            return response
        response.set_body(_gzip.compress(response.body, compresslevel=self._level))
        response.headers["Content-Encoding"] = "gzip"
        response.headers.add_vary("Accept-Encoding")
        return response
