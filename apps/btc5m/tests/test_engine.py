"""Session state machine, driven with a frozen clock and no network."""

from __future__ import annotations

import pytest

from btc5m.engine import Session
from btc5m.errors import TransientSourceError
from btc5m.executors.paper import PaperExecutor
from btc5m.models import Action, Side, bucket_5m
from btc5m.risk import RiskState
from btc5m.sources.price import SimulatedPriceFeed
from btc5m.sources.simulated import SimulatedSource
from btc5m.store import Store


class Clock:
    def __init__(self, start: float) -> None:
        self.now = start

    def __call__(self) -> float:
        return self.now

    def advance(self, seconds: float) -> None:
        self.now += seconds


class FlakySource(SimulatedSource):
    """A market source that fails a configurable number of times."""

    def __init__(self, failures: int) -> None:
        super().__init__()
        self.remaining = failures

    def current_market(self, at):
        if self.remaining > 0:
            self.remaining -= 1
            raise TransientSourceError("simulated outage")
        return super().current_market(at)


@pytest.fixture
def clock() -> Clock:
    return Clock(float(bucket_5m(1788800000) + 300))


def build(profile, tmp_path, clock, *, source=None, equity=100.0):
    store = Store(tmp_path / "t.db")
    session = Session(
        profile=profile, source=source or SimulatedSource(),
        price_feed=SimulatedPriceFeed(), executor=PaperExecutor(),
        risk=RiskState(profile=profile, starting_equity_usd=equity),
        store=store, clock=clock,
    )
    return session, store


def run_for(session, clock, ticks: int, step: float = 2.0):
    for _ in range(ticks):
        session.tick()
        clock.advance(step)


def test_a_full_session_opens_and_closes_positions(demo, tmp_path, clock):
    session, store = build(demo, tmp_path, clock)
    run_for(session, clock, 2000)
    stats = store.stats()
    assert stats["trades"] > 0
    assert stats["volume_usdc"] > 0
    # Every trade must have been closed for a stated reason.
    assert all(t["close_reason"] for t in store.recent_trades(limit=200))


def test_position_is_opened_then_released(demo, tmp_path, clock):
    session, _ = build(demo, tmp_path, clock)
    seen_position = False
    for _ in range(2000):
        snapshot = session.tick()
        if snapshot.position is not None:
            seen_position = True
        elif seen_position:
            assert session.snapshot.last_trade is not None
            return
        clock.advance(2.0)
    pytest.fail("no complete round trip within the simulated window")


def test_snapshot_is_serialisable_and_carries_no_objects(demo, tmp_path, clock):
    import json

    session, _ = build(demo, tmp_path, clock)
    run_for(session, clock, 40)
    payload = session.snapshot.as_dict()
    json.dumps(payload)                       # must not raise
    assert payload["mode"] == "paper"
    assert payload["profile"] == "demo"


def test_source_errors_trip_the_circuit_breaker(demo, tmp_path, clock):
    session, _ = build(demo, tmp_path, clock, source=FlakySource(failures=10))
    for _ in range(3):
        session.tick()
        clock.advance(2.0)
    assert session.risk.consecutive_source_errors >= 3
    assert session.snapshot.block_reason == "source_circuit_breaker"


def test_breaker_clears_once_the_source_recovers(demo, tmp_path, clock):
    session, _ = build(demo, tmp_path, clock, source=FlakySource(failures=3))
    run_for(session, clock, 6)
    assert session.risk.consecutive_source_errors == 0
    assert session.risk.block_reason(clock()) is None


def test_kill_switch_stops_new_entries(demo, tmp_path, clock):
    session, store = build(demo, tmp_path, clock)
    session.risk.engage_kill_switch("test")
    run_for(session, clock, 600)
    assert store.stats()["trades"] == 0
    assert session.snapshot.block_reason == "kill_switch"


def test_daily_trade_cap_is_enforced_end_to_end(demo, tmp_path, clock):
    import dataclasses

    capped = dataclasses.replace(demo, sizing=dataclasses.replace(demo.sizing, max_trades_per_day=2))
    session, store = build(capped, tmp_path, clock)
    run_for(session, clock, 3000)
    assert store.stats()["trades"] == 2


def test_run_stops_on_request(demo, tmp_path, clock):
    session, _ = build(demo, tmp_path, clock)
    snapshot = session.run(max_ticks=5)
    assert snapshot.running is False
    assert snapshot.ticks == 5


def test_deadline_does_not_abandon_an_open_position(demo, tmp_path, clock):
    """A passed entry deadline stops new entries; it never drops a live one."""
    session, _ = build(demo, tmp_path, clock)
    for _ in range(3000):
        session.tick()
        clock.advance(2.0)
        if session.position is not None:
            break
    assert session.position is not None
    snapshot = session.run(max_ticks=1, deadline_ts=clock() - 1)
    assert session.position is not None or snapshot.last_trade is not None
