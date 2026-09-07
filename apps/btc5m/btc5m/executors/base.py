"""Execution engine protocol.

The engine never knows whether it is trading real money. That is decided once,
at startup, by which executor is constructed — so a paper run and a live run
exercise byte-for-byte the same strategy, risk and exit code paths.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Protocol, runtime_checkable

from ..models import Fill, Market, Side


@dataclass(frozen=True)
class OpenRequest:
    market: Market
    side: Side
    limit_price: float
    stake_usd: float
    at: float


@runtime_checkable
class ExecutionEngine(Protocol):
    name: str
    live: bool

    def open(self, request: OpenRequest) -> Optional[Fill]:
        """Buy ``stake_usd`` of ``side``. Return the fill, or ``None`` if unfilled."""

    def close(self, *, market: Market, side: Side, token_id: str, shares: float,
              mark: Optional[float], at: float) -> Optional[Fill]:
        """Sell ``shares``. Return the fill, or ``None`` if it could not be closed."""
