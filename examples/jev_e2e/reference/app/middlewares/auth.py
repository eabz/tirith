"""Bearer token authentication."""

import hmac
from typing import Iterable, Mapping, Optional

from app.pipeline import Context, Middleware, Request, Response


class BearerAuthMiddleware(Middleware):
    name = "auth"

    def __init__(self, tokens: Mapping[str, str], public_paths: Iterable[str] = ("/health", "/public/")):
        self._tokens = dict(tokens)
        self._public = tuple(public_paths)

    def _is_public(self, path: str) -> bool:
        return any(path == p or (p.endswith("/") and path.startswith(p)) for p in self._public)

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        if self._is_public(request.path):
            return None
        scheme, _, token = (request.header("Authorization") or "").partition(" ")
        if scheme.lower() != "bearer" or not token.strip():
            return self._refuse('Bearer realm="app"')
        token = token.strip()
        for known, user in self._tokens.items():
            if hmac.compare_digest(known.encode(), token.encode()):
                ctx.user = user
                return None
        return self._refuse('Bearer realm="app", error="invalid_token"')

    @staticmethod
    def _refuse(challenge: str) -> Response:
        response = Response.text("unauthorized", 401)
        response.headers["WWW-Authenticate"] = challenge
        return response
