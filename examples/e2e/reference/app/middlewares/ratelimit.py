"""Token-bucket rate limiting per client IP."""

import math
import time
from typing import Callable, Dict, Optional, Tuple

from app.pipeline import Context, Middleware, Request, Response


class RateLimitMiddleware(Middleware):
    """429 once a client has used its burst faster than ``rate`` per second."""

    name = "rate_limit"

    def __init__(self, rate: float, burst: int, clock: Callable[[], float] = time.monotonic):
        self._rate = rate
        self._burst = burst
        self._clock = clock
        self._buckets: Dict[str, Tuple[float, float]] = {}

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        now = self._clock()
        tokens, last = self._buckets.get(request.client_ip, (float(self._burst), now))
        tokens = min(float(self._burst), tokens + (now - last) * self._rate)
        if tokens < 1.0:
            self._buckets[request.client_ip] = (tokens, now)
            response = Response.text("rate limit exceeded", 429)
            response.headers["Retry-After"] = str(max(1, math.ceil((1.0 - tokens) / self._rate)))
            return response
        tokens -= 1.0
        self._buckets[request.client_ip] = (tokens, now)
        ctx.values["rate_limit.remaining"] = int(math.floor(tokens))
        return None

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        remaining = ctx.values.get("rate_limit.remaining")
        if remaining is not None:
            response.headers["X-RateLimit-Remaining"] = str(remaining)
        return response
