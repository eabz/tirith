"""Request identifiers."""

import re
import uuid
from typing import Callable, Optional

from app.pipeline import Context, Middleware, Request, Response

_VALID = re.compile(r"[A-Za-z0-9._-]{1,64}")


class RequestIdMiddleware(Middleware):
    name = "request_id"

    def __init__(self, header: str = "X-Request-ID", generator: Optional[Callable[[], str]] = None):
        self._header = header
        self._generator = generator or (lambda: uuid.uuid4().hex)

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        incoming = request.header(self._header)
        ctx.request_id = incoming if incoming and _VALID.fullmatch(incoming) else self._generator()
        return None

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        if ctx.request_id:
            response.headers[self._header] = ctx.request_id
        return response
