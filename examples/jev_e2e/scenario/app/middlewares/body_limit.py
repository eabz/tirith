"""Refuses request bodies above a size limit with 413."""

from typing import Optional

from app.pipeline import Middleware, Request, Response


class MaxBodyMiddleware(Middleware):
    """413 when the body, or the declared Content-Length, exceeds ``max_bytes``."""

    name = "max_body"

    def __init__(self, max_bytes: int):
        if max_bytes <= 0:
            raise ValueError("max_bytes must be positive")
        self._max_bytes = max_bytes

    def process_request(self, request: Request) -> Optional[Response]:
        declared = request.header("Content-Length")
        if declared is not None:
            try:
                if int(declared) > self._max_bytes:
                    return self._too_large()
            except ValueError:
                return Response.text("invalid content-length", 400)
        if len(request.body) > self._max_bytes:
            return self._too_large()
        return None

    def _too_large(self) -> Response:
        return Response.text("payload too large (limit %d bytes)" % self._max_bytes, 413)
