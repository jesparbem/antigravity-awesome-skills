"""Authentication: Argon2id credentials, server-side sessions, lockout.

Design notes
------------
* Passwords are verified with Argon2id and the hash is transparently rehashed
  when parameters change.
* A failed login costs the same time whether or not the user exists — the dummy
  verify keeps the response indistinguishable, so the form cannot be used to
  enumerate accounts.
* Sessions live server-side. The cookie only carries a signed opaque id, so
  logout genuinely revokes access instead of asking the browser to forget.
* Both an idle timeout and an absolute lifetime apply.
"""

from __future__ import annotations

import hmac
import secrets
import threading
import time
from dataclasses import dataclass
from typing import Optional

from argon2 import PasswordHasher
from itsdangerous import BadSignature, SignatureExpired, URLSafeTimedSerializer

from .settings import WebSettings

_hasher = PasswordHasher()
# Verified on every failed attempt so timing does not reveal whether the user
# exists. The value is a real Argon2 hash of a random string.
_DUMMY_HASH = _hasher.hash(secrets.token_urlsafe(16))

SESSION_COOKIE = "btc5m_session"
CSRF_COOKIE = "btc5m_csrf"


def hash_password(password: str) -> str:
    return _hasher.hash(password)


@dataclass
class SessionRecord:
    sid: str
    username: str
    created_at: float
    last_seen: float
    ip: str
    csrf_token: str


class LoginThrottle:
    """Per-key attempt counter with a lockout window."""

    def __init__(self, max_attempts: int, lockout_seconds: int) -> None:
        self.max_attempts = max_attempts
        self.lockout_seconds = lockout_seconds
        self._lock = threading.Lock()
        self._state: dict[str, tuple[int, float]] = {}

    def locked_for(self, key: str, now: Optional[float] = None) -> float:
        """Remaining lockout in seconds (0.0 when not locked)."""
        now = now if now is not None else time.time()
        with self._lock:
            attempts, until = self._state.get(key, (0, 0.0))
        if attempts >= self.max_attempts and until > now:
            return round(until - now, 1)
        return 0.0

    def record_failure(self, key: str, now: Optional[float] = None) -> None:
        now = now if now is not None else time.time()
        with self._lock:
            attempts, until = self._state.get(key, (0, 0.0))
            if until and until <= now:
                attempts = 0
            attempts += 1
            self._state[key] = (
                attempts,
                now + self.lockout_seconds if attempts >= self.max_attempts else until,
            )

    def reset(self, key: str) -> None:
        with self._lock:
            self._state.pop(key, None)


class AuthManager:
    """Owns the single operator account and all live sessions."""

    def __init__(self, settings: WebSettings) -> None:
        self.settings = settings
        self._password_hash = settings.password_hash or hash_password(
            settings.password_plain or settings.generated_password or secrets.token_urlsafe(16)
        )
        self._serializer = URLSafeTimedSerializer(settings.secret_key, salt="btc5m-session")
        self._sessions: dict[str, SessionRecord] = {}
        self._lock = threading.Lock()
        self.throttle = LoginThrottle(settings.login_max_attempts, settings.login_lockout_seconds)

    # -- credentials --------------------------------------------------------- #
    def verify(self, username: str, password: str) -> bool:
        expected = self.settings.username
        # Compare the username in constant time too, then always run a verify.
        user_ok = hmac.compare_digest(username.encode(), expected.encode())
        target = self._password_hash if user_ok else _DUMMY_HASH
        try:
            _hasher.verify(target, password)
        except Exception:  # never leak why verification failed
            return False
        if not user_ok:
            return False
        if _hasher.check_needs_rehash(self._password_hash):
            self._password_hash = hash_password(password)
        return True

    # -- sessions ------------------------------------------------------------ #
    def create_session(self, username: str, ip: str, now: Optional[float] = None) -> tuple[str, SessionRecord]:
        now = now if now is not None else time.time()
        sid = secrets.token_urlsafe(32)
        record = SessionRecord(
            sid=sid, username=username, created_at=now, last_seen=now,
            ip=ip, csrf_token=secrets.token_urlsafe(32),
        )
        with self._lock:
            self._sessions[sid] = record
        return self._serializer.dumps(sid), record

    def resolve(self, cookie: Optional[str], now: Optional[float] = None) -> Optional[SessionRecord]:
        if not cookie:
            return None
        now = now if now is not None else time.time()
        max_age = self.settings.session_max_hours * 3600
        try:
            sid = self._serializer.loads(cookie, max_age=max_age)
        except (BadSignature, SignatureExpired):
            return None

        with self._lock:
            record = self._sessions.get(sid)
            if record is None:
                return None
            if now - record.created_at > max_age:
                del self._sessions[sid]
                return None
            if now - record.last_seen > self.settings.session_idle_minutes * 60:
                del self._sessions[sid]
                return None
            record.last_seen = now
            return record

    def revoke(self, cookie: Optional[str]) -> None:
        if not cookie:
            return
        try:
            sid = self._serializer.loads(cookie, max_age=self.settings.session_max_hours * 3600)
        except (BadSignature, SignatureExpired):
            return
        with self._lock:
            self._sessions.pop(sid, None)

    def revoke_all(self) -> None:
        with self._lock:
            self._sessions.clear()

    def purge_expired(self, now: Optional[float] = None) -> int:
        """Drop timed-out sessions so the registry cannot grow without bound."""
        now = now if now is not None else time.time()
        idle = self.settings.session_idle_minutes * 60
        absolute = self.settings.session_max_hours * 3600
        with self._lock:
            stale = [
                sid for sid, r in self._sessions.items()
                if now - r.last_seen > idle or now - r.created_at > absolute
            ]
            for sid in stale:
                del self._sessions[sid]
        return len(stale)

    @property
    def active_sessions(self) -> int:
        with self._lock:
            return len(self._sessions)

    @staticmethod
    def csrf_ok(record: SessionRecord, submitted: Optional[str]) -> bool:
        if not submitted:
            return False
        return hmac.compare_digest(record.csrf_token, submitted)
