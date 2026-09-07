"""PnL aggregation over the trade store."""

from __future__ import annotations

import datetime as dt
from typing import Any, Optional

from .models import UTC
from .store import Store


def day_start_ts(at: float) -> float:
    moment = dt.datetime.fromtimestamp(at, UTC)
    return moment.replace(hour=0, minute=0, second=0, microsecond=0).timestamp()


def build_report(store: Store, *, at: float, limit: int = 50) -> dict[str, Any]:
    """Overall and same-UTC-day statistics plus the most recent trades."""
    trades = store.recent_trades(limit=limit)
    by_reason: dict[str, int] = {}
    for trade in trades:
        by_reason[trade["close_reason"]] = by_reason.get(trade["close_reason"], 0) + 1

    best: Optional[dict[str, Any]] = max(trades, key=lambda t: t["pnl_usdc"], default=None)
    worst: Optional[dict[str, Any]] = min(trades, key=lambda t: t["pnl_usdc"], default=None)

    return {
        "generated_at": at,
        "overall": store.stats(),
        "today": store.stats(since_ts=day_start_ts(at)),
        "close_reasons": by_reason,
        "best_trade": best,
        "worst_trade": worst,
        "trades": trades,
    }
