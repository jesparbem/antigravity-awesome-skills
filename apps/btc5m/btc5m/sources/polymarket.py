"""Live Polymarket data access.

Changes from v1:

* One pooled ``httpx.Client`` with explicit connect/read timeouts and bounded
  retries, instead of a fresh ``requests`` call plus a fresh ``ClobClient`` (and
  therefore a fresh TCP+TLS handshake) on *every* poll of *every* token.
* Reads the CLOB REST book directly, which carries resting size. v1 used
  ``py_clob_client`` and discarded size, so ``skip_if_top_ask_notional_usd_lt``
  could never be evaluated.
* Falls back across the previous/current/next slot, as ``candidate_slots`` in
  the config always promised but the code never did.
"""

from __future__ import annotations

import json
import logging
from typing import Any, Optional

import httpx

from ..errors import TransientSourceError
from ..models import BUCKET_SECONDS, Market, Quote, Side, bucket_5m, parse_iso

log = logging.getLogger(__name__)

GAMMA_BASE = "https://gamma-api.polymarket.com"
CLOB_BASE = "https://clob.polymarket.com"


def _maybe_json(value: Any) -> Any:
    """Gamma returns some list fields as JSON-encoded strings."""
    if isinstance(value, str):
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            return value
    return value


def _best_level(levels: list[dict[str, Any]], *, best_is_min: bool) -> tuple[Optional[float], Optional[float]]:
    """Return (price, notional_usd) for the best level, or (None, None).

    Notional is summed across every entry at that exact price, which is what a
    marketable order would actually be able to lift.
    """
    parsed: list[tuple[float, float]] = []
    for level in levels or []:
        try:
            price = float(level.get("price"))
            size = float(level.get("size"))
        except (TypeError, ValueError):
            continue
        if price <= 0 or size <= 0:
            continue
        parsed.append((price, size))
    if not parsed:
        return None, None
    best = min(p for p, _ in parsed) if best_is_min else max(p for p, _ in parsed)
    size = sum(s for p, s in parsed if p == best)
    return best, round(best * size, 6)


class PolymarketSource:
    """Gamma (market metadata) + CLOB (order book) reader."""

    name = "polymarket"

    def __init__(
        self,
        *,
        gamma_base: str = GAMMA_BASE,
        clob_base: str = CLOB_BASE,
        timeout_sec: float = 8.0,
        retries: int = 2,
        client: Optional[httpx.Client] = None,
    ) -> None:
        self.gamma_base = gamma_base.rstrip("/")
        self.clob_base = clob_base.rstrip("/")
        self.retries = max(0, int(retries))
        self._client = client or httpx.Client(
            timeout=httpx.Timeout(timeout_sec, connect=min(4.0, timeout_sec)),
            limits=httpx.Limits(max_keepalive_connections=8, max_connections=16),
            headers={"User-Agent": "btc5m/2.0 (+https://github.com/Novals83/5min-btc-polymarket)"},
            follow_redirects=True,
        )

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> "PolymarketSource":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # -- HTTP ---------------------------------------------------------------- #
    def _get(self, url: str, params: Optional[dict[str, Any]] = None) -> Any:
        last: Optional[Exception] = None
        for attempt in range(self.retries + 1):
            try:
                response = self._client.get(url, params=params)
                response.raise_for_status()
                return response.json()
            except (httpx.HTTPError, json.JSONDecodeError) as exc:
                last = exc
                log.debug("GET %s failed (attempt %d/%d): %s", url, attempt + 1, self.retries + 1, exc)
        raise TransientSourceError(f"GET {url} failed after {self.retries + 1} attempts: {last}") from last

    # -- markets ------------------------------------------------------------- #
    def _event(self, slug: str) -> Optional[dict[str, Any]]:
        data = self._get(f"{self.gamma_base}/events", params={"slug": slug})
        if isinstance(data, list) and data:
            return data[0]
        return None

    def _market_from_event(self, event: dict[str, Any], slug: str, at: float) -> Optional[Market]:
        markets = event.get("markets") or []
        if not markets:
            return None
        raw = markets[0]

        outcomes = _maybe_json(raw.get("outcomes")) or []
        token_ids = _maybe_json(raw.get("clobTokenIds")) or []
        if not isinstance(token_ids, list) or len(token_ids) < 2:
            return None

        up_index, down_index = 0, 1
        labels = [str(o).lower() for o in outcomes[:2]] if isinstance(outcomes, list) else []
        if len(labels) >= 2 and ("up" in labels[1] or "yes" in labels[1]):
            up_index, down_index = 1, 0

        end_ts = parse_iso(str(raw.get("endDate") or raw.get("endDateIso") or ""))
        if end_ts is None:
            return None

        return Market(
            slug=str(raw.get("slug") or slug),
            up_token=str(token_ids[up_index]),
            down_token=str(token_ids[down_index]),
            end_ts=end_ts,
            active=raw.get("active") is not False,
            closed=raw.get("closed") is True,
        )

    def current_market(self, at: float) -> Optional[Market]:
        """Resolve the tradable slot closest to now, trying current then next."""
        current = bucket_5m(at)
        for bucket in (current, current + BUCKET_SECONDS):
            slug = f"btc-updown-5m-{bucket}"
            event = self._event(slug)
            if not event:
                continue
            market = self._market_from_event(event, slug, at)
            if market and market.tradable(at):
                return market
        return None

    # -- book ---------------------------------------------------------------- #
    def _book(self, token_id: str) -> tuple[Optional[float], Optional[float], Optional[float]]:
        """Return (best_bid, best_ask, best_ask_notional_usd) for one token."""
        data = self._get(f"{self.clob_base}/book", params={"token_id": token_id})
        if not isinstance(data, dict):
            return None, None, None
        best_bid, _ = _best_level(data.get("bids") or [], best_is_min=False)
        best_ask, ask_notional = _best_level(data.get("asks") or [], best_is_min=True)
        return best_bid, best_ask, ask_notional

    def quote(self, market: Market, at: float) -> Optional[Quote]:
        up_bid, up_ask, up_notional = self._book(market.up_token)
        down_bid, down_ask, down_notional = self._book(market.down_token)
        if up_ask is None and down_ask is None:
            return None
        return Quote(
            ts=at,
            up_bid=up_bid, up_ask=up_ask, up_ask_notional=up_notional,
            down_bid=down_bid, down_ask=down_ask, down_ask_notional=down_notional,
        )

    def mark_price(self, market: Market, side: Side, at: float) -> Optional[float]:
        """Best bid of the held side — what the position could actually be sold at.

        v1 marked open positions against the Gamma *outcome price*, an
        indicative mid that ignores the book, so stop-losses fired on prices
        nobody was bidding.
        """
        token = market.up_token if side is Side.UP else market.down_token
        best_bid, _, _ = self._book(token)
        return best_bid
