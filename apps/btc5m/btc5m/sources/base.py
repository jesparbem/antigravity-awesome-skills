"""Source protocols.

Both front-ends and the engine depend on these, never on a concrete client, so
the simulator can stand in for Polymarket in tests, demos and the published
dashboard without a single conditional in the engine.
"""

from __future__ import annotations

from typing import Optional, Protocol, runtime_checkable

from ..models import Impulse, Market, Quote, Side


@runtime_checkable
class MarketSource(Protocol):
    """Resolves the current BTC 5m market and its top of book."""

    name: str

    def current_market(self, at: float) -> Optional[Market]:
        ...

    def quote(self, market: Market, at: float) -> Optional[Quote]:
        ...

    def mark_price(self, market: Market, side: Side, at: float) -> Optional[float]:
        """Best bid of ``side`` — the price an open position could exit at."""
        ...


@runtime_checkable
class PriceFeed(Protocol):
    """Supplies BTC spot samples for the impulse filter."""

    name: str

    def sample(self, at: float) -> Optional[float]:
        ...

    def impulse(self, at: float) -> Optional[Impulse]:
        ...
