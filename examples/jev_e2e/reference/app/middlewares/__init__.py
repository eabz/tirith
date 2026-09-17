"""Middlewares shipped with the app. ``app.router.build_pipeline`` wires them."""

from app.middlewares.access_log import AccessLogMiddleware
from app.middlewares.auth import BearerAuthMiddleware
from app.middlewares.body_limit import MaxBodyMiddleware
from app.middlewares.cors import CorsMiddleware
from app.middlewares.gzip import GzipMiddleware
from app.middlewares.ratelimit import RateLimitMiddleware
from app.middlewares.request_id import RequestIdMiddleware
from app.middlewares.security import SecurityHeadersMiddleware
from app.middlewares.slash import TrailingSlashMiddleware
from app.middlewares.timing import TimingMiddleware

__all__ = [
    "AccessLogMiddleware",
    "BearerAuthMiddleware",
    "CorsMiddleware",
    "GzipMiddleware",
    "MaxBodyMiddleware",
    "RateLimitMiddleware",
    "RequestIdMiddleware",
    "SecurityHeadersMiddleware",
    "TimingMiddleware",
    "TrailingSlashMiddleware",
]
