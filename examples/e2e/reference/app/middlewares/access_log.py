"""One JSON line per request."""

import json
import logging
import time
from typing import Callable, Optional

from app.pipeline import Context, Middleware, Request, Response


class AccessLogMiddleware(Middleware):
    name = "access_log"

    def __init__(self, logger: Optional[logging.Logger] = None, clock: Callable[[], float] = time.monotonic):
        self._logger = logger or logging.getLogger("app.access")
        self._clock = clock

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        ctx.values["access_log.start"] = self._clock()
        return None

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        start = ctx.values.get("access_log.start", self._clock())
        fields = {
            "method": request.method,
            "path": request.path,
            "status": response.status,
            "duration_ms": round((self._clock() - float(start)) * 1000.0, 1),  # type: ignore[arg-type]
            "request_id": ctx.request_id,
            "user": ctx.user,
            "client_ip": request.client_ip,
        }
        self._logger.info(json.dumps(fields, sort_keys=True))
        return response
