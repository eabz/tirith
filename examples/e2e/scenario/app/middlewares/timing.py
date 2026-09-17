"""Adds ``X-Response-Time`` to every response."""

import time
from typing import Callable, Optional

from app.pipeline import Middleware, Request, Response

_START = "timing.start"


class TimingMiddleware(Middleware):
    """Measures time spent inside the middlewares added after this one."""

    name = "timing"

    def __init__(self, clock: Callable[[], float] = time.monotonic):
        self._clock = clock

    def process_request(self, request: Request) -> Optional[Response]:
        request.extras[_START] = self._clock()
        return None

    def process_response(self, request: Request, response: Response) -> Response:
        start = request.extras.get(_START)
        if isinstance(start, float) or isinstance(start, int):
            elapsed_ms = (self._clock() - start) * 1000.0
            response.headers["X-Response-Time"] = "%.1fms" % elapsed_ms
        return response
