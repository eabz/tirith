"""Routing and pipeline assembly.

``Router`` maps method and path patterns to handlers. ``build_pipeline``
is the one place middlewares are registered and ordered: the first one
added is the outermost, so it sees every request first and every
response last, including responses short-circuited by later middlewares.
"""

import re
from dataclasses import dataclass
from typing import Dict, List, Optional, Pattern, Tuple

from app.config import Config
from app.middlewares import (
    AccessLogMiddleware,
    BearerAuthMiddleware,
    CorsMiddleware,
    GzipMiddleware,
    MaxBodyMiddleware,
    RateLimitMiddleware,
    RequestIdMiddleware,
    SecurityHeadersMiddleware,
    TimingMiddleware,
    TrailingSlashMiddleware,
)
from app.pipeline import Context, Handler, HttpError, Pipeline, Request, Response

_PARAM = re.compile(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\}")


@dataclass
class Route:
    method: str
    pattern: str
    handler: Handler
    name: str
    regex: Pattern[str]

    def match(self, path: str) -> Optional[Dict[str, str]]:
        found = self.regex.fullmatch(path)
        return found.groupdict() if found else None


def _compile(pattern: str) -> Pattern[str]:
    parts: List[str] = []
    last = 0
    for param in _PARAM.finditer(pattern):
        parts.append(re.escape(pattern[last : param.start()]))
        parts.append("(?P<%s>[^/]+)" % param.group(1))
        last = param.end()
    parts.append(re.escape(pattern[last:]))
    return re.compile("".join(parts))


class Router:
    """Resolves a request to a handler; is itself a pipeline handler."""

    def __init__(self) -> None:
        self._routes: List[Route] = []

    def add(self, method: str, pattern: str, handler: Handler, name: Optional[str] = None) -> None:
        route = Route(method.upper(), pattern, handler, name or pattern, _compile(pattern))
        self._routes.append(route)

    def get(self, pattern: str, name: Optional[str] = None):
        def register(handler: Handler) -> Handler:
            self.add("GET", pattern, handler, name)
            return handler

        return register

    def post(self, pattern: str, name: Optional[str] = None):
        def register(handler: Handler) -> Handler:
            self.add("POST", pattern, handler, name)
            return handler

        return register

    def resolve(self, method: str, path: str) -> Tuple[Route, Dict[str, str]]:
        allowed: List[str] = []
        lookup = "GET" if method == "HEAD" else method
        for route in self._routes:
            params = route.match(path)
            if params is None:
                continue
            if route.method == lookup:
                return route, params
            allowed.append(route.method)
        if allowed:
            raise HttpError(405, "method not allowed; use %s" % ", ".join(sorted(set(allowed))))
        raise HttpError(404, "no route for %s" % path)

    def __call__(self, request: Request, ctx: Context) -> Response:
        try:
            route, params = self.resolve(request.method, request.path)
        except HttpError as err:
            response = Response.text(err.message, err.status)
            if err.status == 405:
                response.headers["Allow"] = err.message.split("use ", 1)[-1]
            return response
        ctx.route = route.name
        ctx.params = params
        response = route.handler(request, ctx)
        if request.method == "HEAD":
            response.body = b""
        return response


def build_pipeline(config: Optional[Config] = None, router: Optional[Router] = None) -> Pipeline:
    """The application's pipeline, outermost middleware first."""
    config = config or Config()
    if router is None:
        from app.handlers import default_router

        router = default_router()
    pipeline = Pipeline(router, clock=config.clock)
    pipeline.add(RequestIdMiddleware(header=config.request_id_header))
    if config.access_log:
        pipeline.add(AccessLogMiddleware(clock=config.clock))
    if config.redirect_trailing_slash:
        pipeline.add(TrailingSlashMiddleware())
    pipeline.add(TimingMiddleware(clock=config.clock))
    if config.cors_allowed_origins:
        pipeline.add(CorsMiddleware(config.cors_allowed_origins))
    if config.rate_limit_per_second > 0:
        pipeline.add(RateLimitMiddleware(config.rate_limit_per_second, config.rate_limit_burst, clock=config.clock))
    if config.api_tokens:
        pipeline.add(BearerAuthMiddleware(config.api_tokens))
    if config.max_body_bytes:
        pipeline.add(MaxBodyMiddleware(config.max_body_bytes))
    if config.security_headers:
        pipeline.add(SecurityHeadersMiddleware())
    if config.gzip_min_size > 0:
        pipeline.add(GzipMiddleware(min_size=config.gzip_min_size))
    return pipeline
