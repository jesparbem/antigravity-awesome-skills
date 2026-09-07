"""Immutable value objects shared by every layer.

Everything that crosses a module boundary is a frozen dataclass with an explicit
``as_dict`` so the TUI, the web API and the SQLite store all serialise the same
shape. Times are UNIX seconds (float, UTC) everywhere; ISO strings only ever
appear at the edges.
"""

from __future__ import annotations

import datetime as dt
import enum
import math
from dataclasses import asdict, dataclass, field
from typing import Any, Optional

UTC = dt.timezone.utc

BUCKET_SECONDS = 300
"""Length of a Polymarket BTC Up/Down slot."""


def now_ts() -> float:
    """Current UTC time as UNIX seconds. Injected into pure code for tests."""
    return dt.datetime.now(UTC).timestamp()


def iso(ts: float) -> str:
    """Format a UNIX timestamp as a ``Z``-suffixed ISO-8601 string."""
    return dt.datetime.fromtimestamp(ts, UTC).isoformat().replace("+00:00", "Z")


def parse_iso(value: str) -> Optional[float]:
    """Parse an ISO-8601 timestamp, returning ``None`` instead of raising."""
    if not value:
        return None
    try:
        return dt.datetime.fromisoformat(str(value).replace("Z", "+00:00")).timestamp()
    except (ValueError, TypeError):
        return None


def bucket_5m(ts: float) -> int:
    """Floor a timestamp to the start of its 5-minute slot."""
    return int(ts) - (int(ts) % BUCKET_SECONDS)


class Side(str, enum.Enum):
    UP = "UP"
    DOWN = "DOWN"

    @property
    def opposite(self) -> "Side":
        return Side.DOWN if self is Side.UP else Side.UP


class Action(str, enum.Enum):
    ENTER = "enter"
    WAIT = "wait"
    SKIP = "skip"


@dataclass(frozen=True)
class Market:
    """A single BTC 5m Up/Down market resolved from the Gamma API."""

    slug: str
    up_token: str
    down_token: str
    end_ts: float
    active: bool = True
    closed: bool = False

    def seconds_left(self, at: float) -> float:
        return self.end_ts - at

    def tradable(self, at: float) -> bool:
        return self.active and not self.closed and self.seconds_left(at) > 0

    def as_dict(self) -> dict[str, Any]:
        return {**asdict(self), "end_iso": iso(self.end_ts)}


@dataclass(frozen=True)
class Quote:
    """Top of book for both sides of one market, as of ``ts``.

    ``*_ask_notional`` is the USD depth resting at the best ask, used by the
    liquidity guard. ``None`` means "unknown", which the guard treats as a miss
    rather than silently passing.
    """

    ts: float
    up_bid: Optional[float] = None
    up_ask: Optional[float] = None
    down_bid: Optional[float] = None
    down_ask: Optional[float] = None
    up_ask_notional: Optional[float] = None
    down_ask_notional: Optional[float] = None

    def ask(self, side: Side) -> Optional[float]:
        return self.up_ask if side is Side.UP else self.down_ask

    def bid(self, side: Side) -> Optional[float]:
        return self.up_bid if side is Side.UP else self.down_bid

    def ask_notional(self, side: Side) -> Optional[float]:
        return self.up_ask_notional if side is Side.UP else self.down_ask_notional

    def spread(self, side: Side) -> Optional[float]:
        bid, ask = self.bid(side), self.ask(side)
        if bid is None or ask is None:
            return None
        return max(0.0, ask - bid)

    def age_sec(self, at: float) -> float:
        return max(0.0, at - self.ts)

    def as_dict(self) -> dict[str, Any]:
        return {**asdict(self), "ts_iso": iso(self.ts)}


@dataclass(frozen=True)
class Impulse:
    """BTC spot movement observed inside the current 5m slot.

    The published strategy gates entries on a ~$70-$100 move; without a spot
    feed that filter cannot exist, so this is measured from an external price
    source and carried alongside the market quote.
    """

    bucket_start: int
    open_price: float
    last_price: float
    samples: int
    ts: float

    @property
    def move_usd(self) -> float:
        return self.last_price - self.open_price

    @property
    def abs_move_usd(self) -> float:
        return abs(self.move_usd)

    @property
    def direction(self) -> Optional[Side]:
        """Side implied by the move, or ``None`` when it is flat."""
        if math.isclose(self.move_usd, 0.0, abs_tol=1e-9):
            return None
        return Side.UP if self.move_usd > 0 else Side.DOWN

    def as_dict(self) -> dict[str, Any]:
        return {
            **asdict(self),
            "move_usd": round(self.move_usd, 2),
            "direction": self.direction.value if self.direction else None,
        }


@dataclass(frozen=True)
class Decision:
    """Outcome of one strategy evaluation.

    ``reason`` is a stable machine code (``price_below_threshold``,
    ``spread_too_wide``, ...). The TUI and web UI group by it, so it must never
    be a free-form sentence.
    """

    action: Action
    reason: str
    side: Optional[Side] = None
    price: Optional[float] = None
    stake_usd: float = 0.0
    hedge_usd: float = 0.0
    detail: dict[str, Any] = field(default_factory=dict)

    @property
    def entering(self) -> bool:
        return self.action is Action.ENTER

    def as_dict(self) -> dict[str, Any]:
        return {
            "action": self.action.value,
            "reason": self.reason,
            "side": self.side.value if self.side else None,
            "price": self.price,
            "stake_usd": round(self.stake_usd, 4),
            "hedge_usd": round(self.hedge_usd, 4),
            "detail": self.detail,
        }


@dataclass(frozen=True)
class Fill:
    """A completed open or close leg."""

    ts: float
    side: Side
    token_id: str
    price: float
    shares: float
    usdc: float
    order_id: Optional[str] = None
    tx_hash: Optional[str] = None
    simulated: bool = False

    def as_dict(self) -> dict[str, Any]:
        return {**asdict(self), "side": self.side.value, "ts_iso": iso(self.ts)}


@dataclass
class Position:
    """An open position and everything needed to close it."""

    market_slug: str
    market_end_ts: float
    side: Side
    token_id: str
    entry_price: float
    shares: float
    cost_usdc: float
    opened_at: float
    stop_loss_price: float
    hedge: Optional[Fill] = None
    open_fill: Optional[Fill] = None

    def unrealised(self, mark: Optional[float]) -> Optional[float]:
        if mark is None:
            return None
        return round(mark * self.shares - self.cost_usdc, 6)

    def as_dict(self) -> dict[str, Any]:
        return {
            "market_slug": self.market_slug,
            "market_end_iso": iso(self.market_end_ts),
            "side": self.side.value,
            "token_id": self.token_id,
            "entry_price": self.entry_price,
            "shares": round(self.shares, 6),
            "cost_usdc": round(self.cost_usdc, 6),
            "opened_at": iso(self.opened_at),
            "stop_loss_price": round(self.stop_loss_price, 6),
            "hedge": self.hedge.as_dict() if self.hedge else None,
        }


@dataclass(frozen=True)
class TradeResult:
    """A finished round trip, persisted and aggregated into the PnL report."""

    market_slug: str
    side: Side
    profile: str
    opened_at: float
    closed_at: float
    entry_price: float
    exit_price: Optional[float]
    shares: float
    cost_usdc: float
    proceeds_usdc: float
    close_reason: str
    simulated: bool
    hedge_usdc: float = 0.0

    @property
    def pnl_usdc(self) -> float:
        return round(self.proceeds_usdc - self.cost_usdc - self.hedge_usdc, 6)

    def as_dict(self) -> dict[str, Any]:
        return {
            **asdict(self),
            "side": self.side.value,
            "opened_at": iso(self.opened_at),
            "closed_at": iso(self.closed_at),
            "pnl_usdc": self.pnl_usdc,
        }
