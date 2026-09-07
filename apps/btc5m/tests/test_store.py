"""Persistence and PnL aggregation."""

from __future__ import annotations

import pytest

from btc5m.models import Side, TradeResult
from btc5m.reporting import build_report, day_start_ts
from btc5m.store import Store

from conftest import T0


def trade(pnl: float, at: float, reason: str = "time_exit") -> TradeResult:
    return TradeResult(
        market_slug=f"slot-{int(at)}", side=Side.UP, profile="demo",
        opened_at=at - 120, closed_at=at, entry_price=0.70, exit_price=0.70,
        shares=7.0, cost_usdc=5.0, proceeds_usdc=5.0 + pnl,
        close_reason=reason, simulated=True,
    )


@pytest.fixture
def store(tmp_path) -> Store:
    s = Store(tmp_path / "trades.db")
    yield s
    s.close()


def test_pnl_accounts_for_the_hedge_leg(store):
    result = TradeResult(
        market_slug="s", side=Side.UP, profile="demo",
        opened_at=T0 - 100, closed_at=T0, entry_price=0.7, exit_price=0.9,
        shares=7.0, cost_usdc=5.0, proceeds_usdc=6.3,
        close_reason="time_exit", simulated=True, hedge_usdc=1.0,
    )
    assert result.pnl_usdc == pytest.approx(0.3)


def test_stats_aggregate_wins_and_losses(store):
    store.record_trade(trade(1.0, T0))
    store.record_trade(trade(-0.5, T0 + 300))
    store.record_trade(trade(2.0, T0 + 600))
    stats = store.stats()
    assert stats == {
        "trades": 3, "wins": 2, "losses": 1,
        "win_rate_pct": pytest.approx(66.67), "pnl_usdc": pytest.approx(2.5),
        "volume_usdc": pytest.approx(15.0),
    }


def test_stats_of_an_empty_store(store):
    assert store.stats()["trades"] == 0
    assert store.stats()["win_rate_pct"] is None


def test_today_filter_excludes_yesterday(store):
    store.record_trade(trade(5.0, T0 - 86_400))
    store.record_trade(trade(-1.0, T0))
    today = store.stats(since_ts=day_start_ts(T0))
    assert today["trades"] == 1
    assert today["pnl_usdc"] == pytest.approx(-1.0)
    assert store.stats()["trades"] == 2


def test_trades_come_back_newest_first(store):
    for i in range(5):
        store.record_trade(trade(float(i), T0 + i * 300))
    slugs = [t["market_slug"] for t in store.recent_trades(limit=3)]
    assert slugs == [f"slot-{int(T0 + i * 300)}" for i in (4, 3, 2)]


def test_report_summarises_close_reasons_and_extremes(store):
    store.record_trade(trade(1.0, T0, "time_exit"))
    store.record_trade(trade(-3.0, T0 + 300, "stop_loss_25pct"))
    store.record_trade(trade(0.5, T0 + 600, "time_exit"))
    report = build_report(store, at=T0 + 900)
    assert report["close_reasons"] == {"time_exit": 2, "stop_loss_25pct": 1}
    assert report["best_trade"]["pnl_usdc"] == pytest.approx(1.0)
    assert report["worst_trade"]["pnl_usdc"] == pytest.approx(-3.0)


def test_events_are_bounded_by_prune(store):
    for i in range(50):
        store.record_event(T0 + i, "heartbeat", {"i": i})
    store.prune_events(keep=10)
    assert len(store.recent_events(limit=100)) == 10


def test_events_round_trip_their_payload(store):
    store.record_event(T0, "opened", {"side": "UP", "price": 0.72})
    event = store.recent_events(limit=1)[0]
    assert event["kind"] == "opened"
    assert event["payload"]["price"] == 0.72


def test_store_survives_reopening(tmp_path):
    path = tmp_path / "persist.db"
    first = Store(path)
    first.record_trade(trade(1.5, T0))
    first.close()
    second = Store(path)
    assert second.stats()["trades"] == 1
    second.close()
