"""Offline market source.

Everything downstream — engine, TUI, web dashboard, tests — runs unchanged
against this. It exists because v1 could not be started at all without a private
sibling repository and funded API credentials, which made the code impossible to
review, demo, or test.

Book prices are derived from the same synthetic spot curve the price feed uses,
so the impulse filter and the order book agree the way they would in a real
momentum market.
"""

from __future__ import annotations

import hashlib
import math
from typing import Optional

from ..models import BUCKET_SECONDS, Market, Quote, Side, bucket_5m
from .price import synthetic_spot


def _jitter(key: str, low: float, high: float) -> float:
    """Deterministic value in [low, high] derived from a string key."""
    digest = hashlib.sha256(key.encode("utf-8")).digest()
    unit = int.from_bytes(digest[:8], "big") / float(1 << 64)
    return low + unit * (high - low)


class SimulatedSource:
    """A fake but self-consistent BTC 5m market."""

    name = "simulated"

    def __init__(self, *, base: float = 104_000.0, seed: int = 7, spread: float = 0.02) -> None:
        self.base = base
        self.seed = seed
        self.spread = spread

    def current_market(self, at: float) -> Market:
        bucket = bucket_5m(at)
        return Market(
            slug=f"btc-updown-5m-{bucket}",
            up_token=f"sim-up-{bucket}",
            down_token=f"sim-down-{bucket}",
            end_ts=float(bucket + BUCKET_SECONDS),
            active=True,
            closed=False,
        )

    def implied_up_probability(self, at: float) -> float:
        """Map the in-slot spot move onto an Up probability.

        A $180 move saturates the book near 0.97/0.03, which is the regime the
        extreme-skew hedge rule is written for.
        """
        bucket = bucket_5m(at)
        move = synthetic_spot(at, base=self.base, seed=self.seed) - synthetic_spot(
            float(bucket), base=self.base, seed=self.seed
        )
        probability = 0.5 + 0.47 * math.tanh(move / 110.0)
        return min(0.97, max(0.03, probability))

    def quote(self, market: Market, at: float) -> Optional[Quote]:
        p_up = self.implied_up_probability(at)
        # Spread widens and narrows over time so the spread guard is exercised
        # rather than being permanently satisfied.
        half = _jitter(f"{market.slug}:spread:{int(at) // 10}", self.spread * 0.4, self.spread * 2.2) / 2.0
        up_ask = min(0.99, round(p_up + half, 3))
        up_bid = max(0.01, round(p_up - half, 3))
        down_ask = min(0.99, round(1.0 - p_up + half, 3))
        down_bid = max(0.01, round(1.0 - p_up - half, 3))
        depth_key = f"{market.slug}:{int(at) // 5}"
        return Quote(
            ts=at,
            up_bid=up_bid, up_ask=up_ask,
            down_bid=down_bid, down_ask=down_ask,
            up_ask_notional=round(_jitter(depth_key + ":up", 25.0, 320.0), 2),
            down_ask_notional=round(_jitter(depth_key + ":down", 25.0, 320.0), 2),
        )

    def mark_price(self, market: Market, side: Side, at: float) -> Optional[float]:
        quote = self.quote(market, at)
        return quote.bid(side) if quote else None
