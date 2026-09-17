"""Request pipeline: the request/response model and the middleware chain.

A request passes through each middleware's ``process_request`` in the
order they were added. The first middleware that returns a ``Response``
short-circuits the chain and the handler is not called. Every middleware
whose ``process_request`` ran then sees the response, in reverse order,
through ``process_response``.

One ``Context`` is created per request and passed to every middleware
hook and to the handler as ``ctx``; it is where per-request data lives.
"""

from __future__ import annotations

import json
import logging
import time
from dataclasses import dataclass, field
from typing import Callable, Dict, Iterable, Iterator, List, Optional, Tuple, Union

log = logging.getLogger("app.pipeline")

REASONS = {
    200: "OK",
    201: "Created",
    204: "No Content",
    304: "Not Modified",
    308: "Permanent Redirect",
    400: "Bad Request",
    401: "Unauthorized",
    403: "Forbidden",
    404: "Not Found",
    405: "Method Not Allowed",
    413: "Payload Too Large",
    429: "Too Many Requests",
    500: "Internal Server Error",
}


class Headers:
    """A case-insensitive header map that keeps the spelling of the last set."""

    def __init__(self, items: Union[None, Dict[str, str], Iterable[Tuple[str, str]]] = None):
        self._items: Dict[str, Tuple[str, str]] = {}
        if items:
            pairs = items.items() if isinstance(items, dict) else items
            for name, value in pairs:
                self[name] = value

    def __getitem__(self, name: str) -> str:
        return self._items[name.lower()][1]

    def __setitem__(self, name: str, value: object) -> None:
        self._items[name.lower()] = (name, str(value))

    def __delitem__(self, name: str) -> None:
        del self._items[name.lower()]

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and name.lower() in self._items

    def __iter__(self) -> Iterator[str]:
        return (name for name, _ in self._items.values())

    def __len__(self) -> int:
        return len(self._items)

    def __repr__(self) -> str:
        return "Headers(%r)" % self.items()

    def get(self, name: str, default: Optional[str] = None) -> Optional[str]:
        entry = self._items.get(name.lower())
        return entry[1] if entry else default

    def pop(self, name: str, default: Optional[str] = None) -> Optional[str]:
        entry = self._items.pop(name.lower(), None)
        return entry[1] if entry else default

    def items(self) -> List[Tuple[str, str]]:
        return list(self._items.values())

    def copy(self) -> "Headers":
        return Headers(self.items())

    def add_vary(self, token: str) -> None:
        """Add ``token`` to the ``Vary`` header unless it is already listed."""
        current = [t.strip() for t in self.get("Vary", "").split(",") if t.strip()]
        if token.lower() not in (t.lower() for t in current):
            current.append(token)
        self["Vary"] = ", ".join(current)


@dataclass
class Request:
    """An incoming HTTP request."""

    method: str
    path: str
    headers: Headers = field(default_factory=Headers)
    body: bytes = b""
    query: Dict[str, str] = field(default_factory=dict)
    client_ip: str = "127.0.0.1"

    def header(self, name: str, default: Optional[str] = None) -> Optional[str]:
        return self.headers.get(name, default)


@dataclass
class Response:
    """An outgoing HTTP response."""

    status: int = 200
    body: bytes = b""
    headers: Headers = field(default_factory=Headers)

    @property
    def reason(self) -> str:
        return REASONS.get(self.status, "Unknown")

    def set_body(self, body: bytes) -> None:
        """Replace the body and keep ``Content-Length`` in step with it."""
        self.body = body
        self.headers["Content-Length"] = str(len(body))

    @classmethod
    def text(cls, text: str, status: int = 200) -> "Response":
        response = cls(status=status)
        response.headers["Content-Type"] = "text/plain; charset=utf-8"
        response.set_body(text.encode("utf-8"))
        return response

    @classmethod
    def json(cls, data: object, status: int = 200) -> "Response":
        response = cls(status=status)
        response.headers["Content-Type"] = "application/json"
        response.set_body(json.dumps(data, sort_keys=True).encode("utf-8"))
        return response

    @classmethod
    def empty(cls, status: int = 204) -> "Response":
        return cls(status=status)


class HttpError(Exception):
    """Raised by handlers to answer with a status and a plain-text message."""

    def __init__(self, status: int, message: Optional[str] = None):
        super().__init__(message or REASONS.get(status, "error"))
        self.status = status
        self.message = message or REASONS.get(status, "error").lower()


@dataclass
class Context:
    """Per-request state created by the pipeline and handed to the handler."""

    started_at: float
    request_id: Optional[str] = None
    user: Optional[str] = None
    route: Optional[str] = None
    params: Dict[str, str] = field(default_factory=dict)
    values: Dict[str, object] = field(default_factory=dict)


class Middleware:
    """Base class for middlewares. Both hooks are optional."""

    name = "middleware"

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        """Return a Response to short-circuit the chain, or None to continue."""
        return None

    def process_response(self, request: Request, response: Response, ctx: Context) -> Response:
        """Return the response to pass outwards (usually the same object)."""
        return response


Handler = Callable[[Request, Context], Response]


class Pipeline:
    """An ordered chain of middlewares in front of one handler."""

    def __init__(
        self,
        handler: Handler,
        middlewares: Iterable[Middleware] = (),
        clock: Callable[[], float] = time.monotonic,
    ):
        self._handler = handler
        self._middlewares: List[Middleware] = list(middlewares)
        self._clock = clock

    @property
    def middlewares(self) -> Tuple[Middleware, ...]:
        return tuple(self._middlewares)

    def add(self, middleware: Middleware) -> None:
        """Append a middleware; it runs after (inside) those already added."""
        self._middlewares.append(middleware)

    def names(self) -> List[str]:
        return [m.name for m in self._middlewares]

    def dispatch(self, request: Request) -> Response:
        ctx = Context(started_at=self._clock())
        ran: List[Middleware] = []
        response: Optional[Response] = None
        for middleware in self._middlewares:
            ran.append(middleware)
            response = middleware.process_request(request, ctx)
            if response is not None:
                break
        if response is None:
            response = self._call_handler(request, ctx)
        for middleware in reversed(ran):
            response = middleware.process_response(request, response, ctx)
        return response

    def _call_handler(self, request: Request, ctx: Context) -> Response:
        try:
            return self._handler(request, ctx)
        except HttpError as err:
            return Response.text(err.message, err.status)
        except Exception:  # noqa: BLE001 - the pipeline is the last line of defense
            log.exception("handler failed: %s %s", request.method, request.path)
            return Response.text("internal server error", 500)
