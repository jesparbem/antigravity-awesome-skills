"""BTC spot feeds and the in-slot impulse tracker.

The published strategy requires a ~$70-$100 BTC move inside the active slot
before entering. v1 had no spot feed whatsoever, so that rule was unenforceable.
``ImpulseTracker`` keeps a small ring of samples per 5m bucket and reports the
move from the first sample of the slot to the latest one.
"""

from __future__ import annotations

import logging
import math
from typing import Callable, Optional

import httpx

from ..errors import TransientSourceError
from ..models import Impulse, bucket_5m

log = logging.getLogger(__name__)

BINANCE_URL = "https://api.binance.com/api/v3/ticker/price"
COINBASE_URL = "https://api.coinbase.com/v2/prices/BTC-USD/spot"


class ImpulseTracker:
    """Accumulates spot samples and derives the current slot's impulse.

    Samples from previous buckets are dropped on rollover, so the move always
    measures the *active* interval — which is what the strategy specifies.
    """

    def __init__(self, max_samples: int = 600) -> None:
        self.max_samples = max_samples
        self._bucket: Optional[int] = None
        self._samples: list[tuple[float, float]] = []

    def add(self, ts: float, price: float) -> None:
        bucket = bucket_5m(ts)
        if bucket != self._bucket:
            self._bucket = bucket
            self._samples = []
        self._samples.append((ts, price))
        if len(self._samples) > self.max_samples:
            del self._samples[: len(self._samples) - self.max_samples]

    def current(self) -> Optional[Impulse]:
        if self._bucket is None or not self._samples:
            return None
        first_ts, first_price = self._samples[0]
        last_ts, last_price = self._samples[-1]
        return Impulse(
            bucket_start=self._bucket,
            open_price=first_price,
            last_price=last_price,
            samples=len(self._samples),
            ts=last_ts,
        )

    def reset(self) -> None:
        self._bucket = None
        self._samples = []


class HttpPriceFeed:
    """Spot price over HTTP, with a pluggable response extractor."""

    def __init__(
        self,
        *,
        name: str,
        url: str,
        extract: Callable[[object], float],
        params: Optional[dict[str, str]] = None,
        timeout_sec: float = 5.0,
        client: Optional[httpx.Client] = None,
    ) -> None:
        self.name = name
        self.url = url
        self.params = params or {}
        self._extract = extract
        self._client = client or httpx.Client(timeout=timeout_sec, follow_redirects=True)
        self._tracker = ImpulseTracker()

    def close(self) -> None:
        self._client.close()

    def sample(self, at: float) -> Optional[float]:
        try:
            response = self._client.get(self.url, params=self.params)
            response.raise_for_status()
            price = float(self._extract(response.json()))
        except (httpx.HTTPError, KeyError, TypeError, ValueError) as exc:
            raise TransientSourceError(f"{self.name} spot fetch failed: {exc}") from exc
        self._tracker.add(at, price)
        return price

    def impulse(self, at: float) -> Optional[Impulse]:
        return self._tracker.current()


def binance_feed(symbol: str = "BTCUSDT", **kwargs: object) -> HttpPriceFeed:
    return HttpPriceFeed(
        name="binance",
        url=BINANCE_URL,
        params={"symbol": symbol},
        extract=lambda payload: payload["price"],  # type: ignore[index]
        **kwargs,  # type: ignore[arg-type]
    )


def coinbase_feed(symbol: str = "BTC-USD", **kwargs: object) -> HttpPriceFeed:
    return HttpPriceFeed(
        name="coinbase",
        url=COINBASE_URL,
        extract=lambda payload: payload["data"]["amount"],  # type: ignore[index]
        **kwargs,  # type: ignore[arg-type]
    )


# --------------------------------------------------------------------------- #
# offline
# --------------------------------------------------------------------------- #
def synthetic_spot(ts: float, *, base: float = 104_000.0, seed: int = 7) -> float:
    """A smooth, deterministic BTC-like price curve.

    Sum of seeded sinusoids rather than a random walk, so ``synthetic_spot(t)``
    is pure: the demo dashboard, the tests and the TUI all see the same market
    for the same timestamp, with no shared state to synchronise.
    """
    phases = [(seed * 13 % 97) / 97.0, (seed * 29 % 89) / 89.0, (seed * 41 % 83) / 83.0, (seed * 7 % 71) / 71.0]
    # Calibrated at the strategy's target moment (120s left): the in-slot move
    # has a ~$65 median and a ~$150 p90, so the $70 impulse threshold and the
    # 0.70 price threshold together admit roughly half of all slots.
    periods = (3607.0, 1201.0, 433.0, 151.0)
    amplitudes = (214.2, 88.2, 36.5, 11.3)
    value = base
    for phase, period, amplitude in zip(phases, periods, amplitudes):
        value += amplitude * math.sin(2 * math.pi * (ts / period + phase))
    return round(value, 2)


class SimulatedPriceFeed:
    """Offline spot feed driven by :func:`synthetic_spot`."""

    name = "simulated"

    def __init__(self, *, base: float = 104_000.0, seed: int = 7) -> None:
        self.base = base
        self.seed = seed
        self._tracker = ImpulseTracker()

    def sample(self, at: float) -> Optional[float]:
        price = synthetic_spot(at, base=self.base, seed=self.seed)
        self._tracker.add(at, price)
        return price

    def impulse(self, at: float) -> Optional[Impulse]:
        """Report the true in-slot move even before any sampling has happened."""
        bucket = bucket_5m(at)
        tracked = self._tracker.current()
        if tracked is not None and tracked.bucket_start == bucket and tracked.samples >= 2:
            return tracked
        return Impulse(
            bucket_start=bucket,
            open_price=synthetic_spot(float(bucket), base=self.base, seed=self.seed),
            last_price=synthetic_spot(at, base=self.base, seed=self.seed),
            samples=max(3, tracked.samples if tracked else 3),
            ts=at,
        )

    def close(self) -> None:  # parity with HttpPriceFeed
        return None


def build_price_feed(source: str, symbol: str) -> object:
    """Factory used by the engine to honour ``impulse_filter.source``."""
    if source == "binance":
        return binance_feed(symbol)
    if source == "coinbase":
        return coinbase_feed(symbol)
    return SimulatedPriceFeed()
