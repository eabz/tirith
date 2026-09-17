"""Redirects ``/items/`` to ``/items`` for safe methods."""

from typing import Optional
from urllib.parse import urlencode

from app.pipeline import Context, Middleware, Request, Response


class TrailingSlashMiddleware(Middleware):
    """308 to the path without its trailing slash, keeping the query string."""

    name = "trailing_slash"

    def process_request(self, request: Request, ctx: Context) -> Optional[Response]:
        if request.method not in ("GET", "HEAD"):
            return None
        if len(request.path) <= 1 or not request.path.endswith("/"):
            return None
        location = request.path.rstrip("/") or "/"
        if request.query:
            location += "?" + urlencode(sorted(request.query.items()))
        response = Response.empty(308)
        response.headers["Location"] = location
        return response
