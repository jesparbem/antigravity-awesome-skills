"""Structured logging with secret redaction.

Trading credentials live in the process environment, and v1's failure path
printed raw subprocess output — which included whatever the child echoed. Every
record here passes through a filter that masks anything shaped like a private
key, API secret or bearer token before it reaches a handler.
"""

from __future__ import annotations

import json
import logging
import os
import re
import sys
from typing import Any, Iterable

SECRET_ENV_HINTS = ("KEY", "SECRET", "PASSPHRASE", "TOKEN", "PASSWORD", "MNEMONIC", "SEED")

_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"0x[a-fA-F0-9]{40,}"),                      # private keys / long hex
    re.compile(r"(?i)\b(?:api[_-]?(?:key|secret)|passphrase|password|token)\b\s*[:=]\s*\S+"),
    re.compile(r"\b[A-Za-z0-9+/]{40,}={0,2}\b"),            # base64-ish blobs
)

MASK = "***REDACTED***"


def secret_values_from_env(env: Iterable[tuple[str, str]] | None = None) -> list[str]:
    """Collect live secret values so they can be masked literally."""
    items = env if env is not None else os.environ.items()
    out = []
    for name, value in items:
        if value and len(value) >= 8 and any(hint in name.upper() for hint in SECRET_ENV_HINTS):
            out.append(value)
    return out


def redact(text: str, extra: Iterable[str] = ()) -> str:
    """Mask secrets in ``text`` — both known literal values and shape matches."""
    if not text:
        return text
    for value in extra:
        if value:
            text = text.replace(value, MASK)
    for pattern in _PATTERNS:
        text = pattern.sub(MASK, text)
    return text


class RedactingFilter(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self._literals = secret_values_from_env()

    def filter(self, record: logging.LogRecord) -> bool:
        try:
            record.msg = redact(str(record.getMessage()), self._literals)
            record.args = ()
        except Exception:  # logging must never raise
            record.msg = "<unloggable record>"
            record.args = ()
        return True


class JsonFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, Any] = {
            "ts": self.formatTime(record, "%Y-%m-%dT%H:%M:%S%z"),
            "level": record.levelname,
            "logger": record.name,
            "msg": record.getMessage(),
        }
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        return json.dumps(payload, ensure_ascii=False)


def configure(level: str = "INFO", *, json_logs: bool = False, stream: Any = None) -> None:
    """Install the root handler. Idempotent — safe to call from CLI and web."""
    root = logging.getLogger()
    for handler in list(root.handlers):
        root.removeHandler(handler)
    handler = logging.StreamHandler(stream or sys.stderr)
    handler.setFormatter(
        JsonFormatter() if json_logs
        else logging.Formatter("%(asctime)s %(levelname)-7s %(name)s: %(message)s", "%H:%M:%S")
    )
    handler.addFilter(RedactingFilter())
    root.addHandler(handler)
    root.setLevel(getattr(logging, level.upper(), logging.INFO))
    logging.getLogger("httpx").setLevel(logging.WARNING)
    logging.getLogger("httpcore").setLevel(logging.WARNING)
