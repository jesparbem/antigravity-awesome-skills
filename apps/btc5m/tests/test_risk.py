"""Account-level caps. None of these existed in v1."""

from __future__ import annotations

import datetime as dt

import pytest

from btc5m.models import UTC, Side, TradeResult
from btc5m.risk import RiskState, utc_day

from conftest import T0


def trade(pnl: float, at: float = T0) -> TradeResult:
    """A closed trade whose PnL is exactly ``pnl``."""
    return TradeResult(
        market_slug="s", side=Side.UP, profile="conservative",
        opened_at=at - 100, closed_at=at, entry_price=0.7, exit_price=0.7,
        shares=7.0, cost_usdc=5.0, proceeds_usdc=5.0 + pnl,
        close_reason="test", simulated=True,
    )


@pytest.fixture
def risk(conservative) -> RiskState:
    return RiskState(profile=conservative, starting_equity_usd=100.0)


def test_clean_state_allows_trading(risk):
    assert risk.block_reason(T0) is None


def test_trade_cap_blocks(risk, conservative):
    for _ in range(conservative.sizing.max_trades_per_day):
        risk.record_trade(trade(0.1))
    assert risk.block_reason(T0) == "max_trades_per_day"


def test_daily_loss_cap_blocks(risk):
    # conservative: 10% of $100 equity = $10.
    risk.record_trade(trade(-9.0))
    assert risk.block_reason(T0) is None
    risk.record_trade(trade(-1.5))
    assert risk.block_reason(T0) == "daily_loss_limit"


def test_kill_switch_blocks_and_releases(risk):
    risk.engage_kill_switch("operator")
    assert risk.block_reason(T0) == "kill_switch"
    risk.release_kill_switch()
    assert risk.block_reason(T0) is None


def test_circuit_breaker_after_consecutive_errors(risk, conservative):
    for _ in range(conservative.safety.skip_if_dns_or_api_errors_consecutive):
        risk.record_source_error()
    assert risk.block_reason(T0) == "source_circuit_breaker"
    risk.record_source_ok()
    assert risk.block_reason(T0) is None


def test_counters_reset_on_utc_day_rollover(risk):
    risk.record_trade(trade(-9.9))
    assert risk.block_reason(T0) is not None or risk.realised_pnl_usdc < 0
    tomorrow = T0 + 86_400
    risk.roll_day(tomorrow)
    assert risk.trades_today == 0
    assert risk.realised_pnl_usdc == 0.0
    assert risk.day == utc_day(tomorrow)


def test_equity_tracks_realised_pnl(risk):
    risk.record_trade(trade(2.5))
    assert risk.equity_usd == pytest.approx(102.5)


def test_seed_restores_todays_ledger(risk):
    """The daily cap must survive a process restart."""
    risk.seed(realised_pnl_usdc=-9.5, trades=4)
    assert risk.trades_today == 4
    assert risk.daily_loss_used_pct == pytest.approx(95.0)
    risk.record_trade(trade(-1.0))
    assert risk.block_reason(T0) == "daily_loss_limit"


def test_daily_loss_used_pct_is_zero_when_profitable(risk):
    risk.record_trade(trade(3.0))
    assert risk.daily_loss_used_pct == 0.0


def test_as_dict_exposes_no_internals(risk):
    keys = set(risk.as_dict())
    assert "_lock" not in keys and "profile" not in keys
