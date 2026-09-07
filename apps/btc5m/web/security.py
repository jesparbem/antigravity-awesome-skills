"""Transport- and request-level hardening.

Three concerns, kept separate from the routes so they cannot be forgotten on a
new endpoint:

* :class:`SecurityHeadersMiddleware` — a strict CSP plus the usual header set.
  The CSP allows no inline script or style at all, which is why the dashboard
  ships real ``.css`` and ``.js`` files rather than inlining them.
* :class:`RateLimitMiddleware` — a per-client token bucket in front of ``/api``,
  so a leaked session cannot be used to hammer the upstream exchange APIs.
* :func:`client_ip` — resolves the peer address, trusting ``X-Forwarded-For``
  only when the operator has said there is a proxy in front.
"""

from __future__ import annotations

import threading
import time
from typing import Optional

from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request
from starlette.responses import JSONResponse, Response

CSP = (
    "default-src 'none'; "
    "script-src 'self'; "
    "style-src 'self'; "
    "img-src 'self' data:; "
    "font-src 'self'; "
    "connect-src 'self'; "
    "form-action 'self'; "
    "base-uri 'none'; "
    "frame-ancestors 'none'"
)


def client_ip(request: Request, trust_proxy: bool = False) -> str:
    if trust_proxy:
        forwarded = request.headers.get("x-forwarded-for")
        if forwarded:
            # Left-most entry is the original client.
            return forwarded.split(",")[0].strip()
    return request.client.host if request.client else "unknown"


class SecurityHeadersMiddleware(BaseHTTPMiddleware):
    def __init__(self, app, *, https_only: bool = True) -> None:
        super().__init__(app)
        self.https_only = https_only

    async def dispatch(self, request: Request, call_next) -> Response:
        response = await call_next(request)
        headers = response.headers
        headers.setdefault("Content-Security-Policy", CSP)
        headers.setdefault("X-Content-Type-Options", "nosniff")
        headers.setdefault("X-Frame-Options", "DENY")
        headers.setdefault("Referrer-Policy", "no-referrer")
        headers.setdefault("Permissions-Policy", "geolocation=(), microphone=(), camera=(), payment=()")
        headers.setdefault("Cross-Origin-Opener-Policy", "same-origin")
        headers.setdefault("Cross-Origin-Resource-Policy", "same-origin")
        headers.setdefault("Cache-Control", "no-store")
        if self.https_only:
            headers.setdefault("Strict-Transport-Security", "max-age=63072000; includeSubDomains")
        return response


class TokenBucket:
    """Fixed-rate bucket, refilled continuously."""

    __slots__ = ("capacity", "refill_per_sec", "tokens", "updated")

    def __init__(self, capacity: int, refill_per_sec: float, now: float) -> None:
        self.capacity = capacity
        self.refill_per_sec = refill_per_sec
        self.tokens = float(capacity)
        self.updated = now

    def take(self, now: float) -> bool:
        self.tokens = min(self.capacity, self.tokens + (now - self.updated) * self.refill_per_sec)
        self.updated = now
        if self.tokens >= 1.0:
            self.tokens -= 1.0
            return True
        return False


class RateLimitMiddleware(BaseHTTPMiddleware):
    """Per-IP rate limit on a path prefix."""

    def __init__(self, app, *, per_minute: int = 240, prefix: str = "/api",
                 trust_proxy: bool = False, max_clients: int = 4096) -> None:
        super().__init__(app)
        self.per_minute = max(1, per_minute)
        self.prefix = prefix
        self.trust_proxy = trust_proxy
        self.max_clients = max_clients
        self._buckets: dict[str, TokenBucket] = {}
        self._lock = threading.Lock()

    def _allow(self, key: str, now: float) -> bool:
        with self._lock:
            bucket = self._buckets.get(key)
            if bucket is None:
                if len(self._buckets) >= self.max_clients:
                    # Bound memory: drop the least recently used bucket.
                    oldest = min(self._buckets, key=lambda k: self._buckets[k].updated)
                    del self._buckets[oldest]
                bucket = TokenBucket(self.per_minute, self.per_minute / 60.0, now)
                self._buckets[key] = bucket
            return bucket.take(now)

    async def dispatch(self, request: Request, call_next) -> Response:
        if not request.url.path.startswith(self.prefix):
            return await call_next(request)
        if not self._allow(client_ip(request, self.trust_proxy), time.time()):
            return JSONResponse(
                {"error": "rate_limited", "detail": "too many requests"},
                status_code=429, headers={"Retry-After": "5"},
            )
        return await call_next(request)
