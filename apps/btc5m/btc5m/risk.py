"""Account-level risk state: daily caps, trade counts, circuit breaker, kill switch.

v1 declared `daily_max_loss_pct`, `max_trades_per_day` and
`skip_if_dns_or_api_errors_consecutive` in YAML and enforced none of them — a
session could lose the account in a loop of retries. This module owns that state
and is consulted before every entry.

The ledger rolls over on UTC calendar day, matching how the caps are described.
"""

from __future__ import annotations

import datetime as dt
import threading
from dataclasses import dataclass, field
from typing import Any, Optional

from .config import Profile
from .models import UTC, TradeResult


def utc_day(at: float) -> str:
    return dt.datetime.fromtimestamp(at, UTC).strftime("%Y-%m-%d")


@dataclass
class RiskState:
    """Mutable, thread-safe risk ledger for one running session."""

    profile: Profile
    starting_equity_usd: float
    day: str = ""
    realised_pnl_usdc: float = 0.0
    trades_today: int = 0
    consecutive_source_errors: int = 0
    kill_switch: bool = False
    kill_switch_reason: str = ""
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False)

    def __post_init__(self) -> None:
        if not self.day:
            self.day = utc_day(dt.datetime.now(UTC).timestamp())

    # -- ledger ------------------------------------------------------------- #
    def roll_day(self, at: float) -> None:
        """Reset per-day counters when the UTC date changes."""
        today = utc_day(at)
        with self._lock:
            if today != self.day:
                self.day = today
                self.realised_pnl_usdc = 0.0
                self.trades_today = 0

    def seed(self, *, realised_pnl_usdc: float, trades: int) -> None:
        """Adopt today's already-recorded results.

        Without this, restarting the process reset ``trades_today`` and the
        realised loss to zero, so the daily loss cap and the trade ceiling could
        be bypassed simply by relaunching — the caps only ever bound one process
        lifetime, which is not what "daily max loss" means.
        """
        with self._lock:
            self.realised_pnl_usdc = round(float(realised_pnl_usdc), 6)
            self.trades_today = max(0, int(trades))

    def record_trade(self, result: TradeResult) -> None:
        with self._lock:
            self.trades_today += 1
            self.realised_pnl_usdc = round(self.realised_pnl_usdc + result.pnl_usdc, 6)

    def record_source_error(self) -> int:
        with self._lock:
            self.consecutive_source_errors += 1
            return self.consecutive_source_errors

    def record_source_ok(self) -> None:
        with self._lock:
            self.consecutive_source_errors = 0

    def engage_kill_switch(self, reason: str) -> None:
        with self._lock:
            self.kill_switch = True
            self.kill_switch_reason = reason

    def release_kill_switch(self) -> None:
        with self._lock:
            self.kill_switch = False
            self.kill_switch_reason = ""

    # -- derived ------------------------------------------------------------ #
    @property
    def equity_usd(self) -> float:
        return round(self.starting_equity_usd + self.realised_pnl_usdc, 6)

    @property
    def daily_loss_limit_usdc(self) -> float:
        return round(self.starting_equity_usd * (self.profile.sizing.daily_max_loss_pct / 100.0), 6)

    @property
    def daily_loss_used_pct(self) -> float:
        limit = self.daily_loss_limit_usdc
        if limit <= 0:
            return 0.0
        used = max(0.0, -self.realised_pnl_usdc)
        return round(min(100.0, used / limit * 100.0), 2)

    def block_reason(self, at: float) -> Optional[str]:
        """Why a new entry is not allowed right now, or ``None`` if it is."""
        self.roll_day(at)
        with self._lock:
            if self.kill_switch:
                return "kill_switch"
            if self.trades_today >= self.profile.sizing.max_trades_per_day:
                return "max_trades_per_day"
            if -self.realised_pnl_usdc >= self.daily_loss_limit_usdc:
                return "daily_loss_limit"
            if self.consecutive_source_errors >= self.profile.safety.skip_if_dns_or_api_errors_consecutive:
                return "source_circuit_breaker"
        return None

    def as_dict(self) -> dict[str, Any]:
        return {
            "day": self.day,
            "starting_equity_usd": round(self.starting_equity_usd, 4),
            "equity_usd": self.equity_usd,
            "realised_pnl_usdc": self.realised_pnl_usdc,
            "trades_today": self.trades_today,
            "max_trades_per_day": self.profile.sizing.max_trades_per_day,
            "daily_loss_limit_usdc": self.daily_loss_limit_usdc,
            "daily_loss_used_pct": self.daily_loss_used_pct,
            "consecutive_source_errors": self.consecutive_source_errors,
            "kill_switch": self.kill_switch,
            "kill_switch_reason": self.kill_switch_reason,
        }
