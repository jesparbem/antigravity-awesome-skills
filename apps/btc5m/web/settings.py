"""Web configuration, read once from the environment.

Every security-relevant default is the *safe* one: read-only controls, secure
cookies assumed, short sessions. Loosening any of them takes an explicit
environment variable, so a careless deploy fails closed.
"""

from __future__ import annotations

import os
import secrets
from dataclasses import dataclass, field
from typing import Optional


def _flag(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.strip().lower() in {"1", "true", "yes", "on"}


def _int(name: str, default: int) -> int:
    try:
        return int(os.environ.get(name, default))
    except (TypeError, ValueError):
        return default


@dataclass
class WebSettings:
    username: str = field(default_factory=lambda: os.environ.get("BTC5M_WEB_USER", "admin"))
    password_hash: Optional[str] = field(default_factory=lambda: os.environ.get("BTC5M_WEB_PASSWORD_HASH"))
    password_plain: Optional[str] = field(default_factory=lambda: os.environ.get("BTC5M_WEB_PASSWORD"))
    secret_key: str = field(default_factory=lambda: os.environ.get("BTC5M_WEB_SECRET", ""))

    # Controls are read-only unless deliberately enabled.
    read_only: bool = field(default_factory=lambda: _flag("BTC5M_WEB_READONLY", True))
    # Serving over TLS (directly or behind a terminating proxy) enables Secure
    # cookies and HSTS. Left on by default so a plaintext deploy is the choice
    # that has to be made explicitly.
    https_only: bool = field(default_factory=lambda: _flag("BTC5M_WEB_HTTPS", True))

    session_idle_minutes: int = field(default_factory=lambda: _int("BTC5M_WEB_SESSION_IDLE_MIN", 30))
    session_max_hours: int = field(default_factory=lambda: _int("BTC5M_WEB_SESSION_MAX_HOURS", 8))
    login_max_attempts: int = field(default_factory=lambda: _int("BTC5M_WEB_LOGIN_MAX_ATTEMPTS", 5))
    login_lockout_seconds: int = field(default_factory=lambda: _int("BTC5M_WEB_LOGIN_LOCKOUT_SEC", 300))
    api_rate_per_minute: int = field(default_factory=lambda: _int("BTC5M_WEB_API_RATE_PER_MIN", 240))

    allowed_hosts: list[str] = field(
        default_factory=lambda: [
            h.strip() for h in os.environ.get("BTC5M_WEB_ALLOWED_HOSTS", "*").split(",") if h.strip()
        ]
    )
    trust_proxy_headers: bool = field(default_factory=lambda: _flag("BTC5M_WEB_TRUST_PROXY", False))

    # Session defaults
    profile: str = field(default_factory=lambda: os.environ.get("BTC5M_WEB_PROFILE", "demo"))
    source: str = field(default_factory=lambda: os.environ.get("BTC5M_WEB_SOURCE", "simulated"))
    equity_usd: float = field(default_factory=lambda: float(os.environ.get("BTC5M_WEB_EQUITY", "100")))
    db_path: str = field(default_factory=lambda: os.environ.get("BTC5M_DB", "runtime/btc5m.db"))

    generated_password: Optional[str] = None
    generated_secret: bool = False

    def __post_init__(self) -> None:
        if not self.secret_key:
            # A random key means sessions do not survive a restart, which is the
            # safe failure: it can never be a predictable shared default.
            self.secret_key = secrets.token_urlsafe(48)
            self.generated_secret = True
        if not self.password_hash and not self.password_plain:
            self.generated_password = secrets.token_urlsafe(12)

    @property
    def cookie_secure(self) -> bool:
        return self.https_only

    def public_dict(self) -> dict[str, object]:
        """Only non-sensitive settings — never the hash, key or password."""
        return {
            "read_only": self.read_only,
            "https_only": self.https_only,
            "profile": self.profile,
            "source": self.source,
            "session_idle_minutes": self.session_idle_minutes,
            "session_max_hours": self.session_max_hours,
        }
