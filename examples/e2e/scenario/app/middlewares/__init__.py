"""Middlewares shipped with the app. ``app.router.build_pipeline`` wires them."""

from app.middlewares.body_limit import MaxBodyMiddleware
from app.middlewares.security import SecurityHeadersMiddleware
from app.middlewares.slash import TrailingSlashMiddleware
from app.middlewares.timing import TimingMiddleware

__all__ = [
    "MaxBodyMiddleware",
    "SecurityHeadersMiddleware",
    "TimingMiddleware",
    "TrailingSlashMiddleware",
]
