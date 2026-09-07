"""Exception hierarchy.

A narrow set of typed errors lets the engine distinguish *transient* problems
(retry, count towards the circuit breaker) from *fatal* ones (stop the session).
The original script swallowed every exception into a bare ``except Exception``,
which made a misconfigured profile look identical to a network blip.
"""

from __future__ import annotations


class Btc5mError(Exception):
    """Base class for every error raised by this package."""


class ConfigError(Btc5mError):
    """The profile file is missing, malformed, or fails validation."""


class TransientSourceError(Btc5mError):
    """Market data was temporarily unavailable. Retry, then trip the breaker."""


class ExecutionError(Btc5mError):
    """The execution engine could not place or close an order."""


class KillSwitchEngaged(Btc5mError):
    """Trading is administratively disabled."""
