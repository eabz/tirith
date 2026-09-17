"""Application settings.

Every feature flag lives here with a default that keeps the feature off
or conservative. ``Config.from_env`` reads ``APP_*`` environment
variables for deployments; tests build ``Config(...)`` directly.
"""

import time
from dataclasses import dataclass, field, fields
from typing import Callable, Dict, Mapping, Optional, Tuple


def _parse_bool(value: str) -> bool:
    lowered = value.strip().lower()
    if lowered in ("1", "true", "yes", "on"):
        return True
    if lowered in ("0", "false", "no", "off", ""):
        return False
    raise ValueError("not a boolean: %r" % value)


@dataclass
class Config:
    """Settings consumed by ``app.router.build_pipeline``."""

    # Largest accepted request body in bytes; 0 disables the check.
    max_body_bytes: int = 1_048_576
    # Adds X-Content-Type-Options, Referrer-Policy and X-Frame-Options.
    security_headers: bool = True
    # Redirect GET /items/ to /items.
    redirect_trailing_slash: bool = True
    # Monotonic clock in seconds. Tests inject a fake one.
    clock: Callable[[], float] = field(default=time.monotonic, repr=False, compare=False)
    debug: bool = False
    rate_limit_per_second: float = 0.0
    rate_limit_burst: int = 10
    cors_allowed_origins: Tuple[str, ...] = ()
    request_id_header: str = "X-Request-ID"
    api_tokens: Dict[str, str] = field(default_factory=dict)
    access_log: bool = False
    gzip_min_size: int = 0

    @classmethod
    def from_env(cls, environ: Optional[Mapping[str, str]] = None) -> "Config":
        """Build a Config from ``APP_<FIELD>`` variables; unknown ones are ignored."""
        import os

        environ = os.environ if environ is None else environ
        values: Dict[str, object] = {}
        for f in fields(cls):
            if f.name == "clock":
                continue
            raw = environ.get("APP_" + f.name.upper())
            if raw is None:
                continue
            if f.type in (bool, "bool"):
                values[f.name] = _parse_bool(raw)
            elif f.type in (int, "int"):
                values[f.name] = int(raw)
            else:
                values[f.name] = raw
        return cls(**values)  # type: ignore[arg-type]
