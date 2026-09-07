"""Entry and exit rules.

These are the checks the v1 runner documented but never enforced, so each one
gets an explicit test.
"""

from __future__ import annotations

import dataclasses

import pytest

from btc5m.models import Action, Impulse, Market, Quote, Side
from btc5m.strategy import evaluate, exit_reason
from btc5m.models import Position

from conftest import T0, make_quote

BUCKET = int(T0)


def impulse_of(move: float, *, samples: int = 10, at: float = T0) -> Impulse:
    return Impulse(bucket_start=BUCKET, open_price=100_000.0,
                   last_price=100_000.0 + move, samples=samples, ts=at)


_UNSET = object()


def run(profile, market, *, at, quote=_UNSET, impulse=_UNSET, equity=100.0):
    """Evaluate with sensible passing defaults; pass ``None`` to remove an input."""
    return evaluate(
        market=market,
        quote=make_quote(at) if quote is _UNSET else quote,
        impulse=impulse_of(120) if impulse is _UNSET else impulse,
        profile=profile, equity_usd=equity, at=at,
    )


# --------------------------------------------------------------------------- #
# entry window
# --------------------------------------------------------------------------- #
def test_enters_inside_the_window(conservative, market):
    at = market.end_ts - 120           # the documented target
    decision = run(conservative, market, at=at)
    assert decision.action is Action.ENTER
    assert decision.side is Side.UP
    assert decision.stake_usd == pytest.approx(5.0)


def test_waits_when_too_early(conservative, market):
    """v1 had no upper bound and would enter here; the strategy says wait."""
    at = market.end_ts - 240
    decision = run(conservative, market, at=at)
    assert decision.action is Action.WAIT
    assert decision.reason == "too_early_to_enter"


def test_skips_when_too_late(conservative, market):
    at = market.end_ts - 30
    assert run(conservative, market, at=at).reason == "too_late_to_enter"


def test_closed_market_is_refused(conservative, market):
    closed = dataclasses.replace(market, closed=True)
    assert run(conservative, closed, at=closed.end_ts - 120).reason == "market_not_tradable"


def test_expired_market_is_refused(conservative, market):
    assert run(conservative, market, at=market.end_ts + 1).reason == "market_not_tradable"


def test_no_market_is_a_wait(conservative):
    decision = evaluate(market=None, quote=None, impulse=None,
                        profile=conservative, equity_usd=100.0, at=T0)
    assert decision.action is Action.WAIT and decision.reason == "no_active_market"


# --------------------------------------------------------------------------- #
# data quality
# --------------------------------------------------------------------------- #
def test_stale_quote_is_refused(conservative, market):
    at = market.end_ts - 120
    stale = dataclasses.replace(make_quote(at), ts=at - 30)
    assert run(conservative, market, at=at, quote=stale).reason == "quote_stale"


def test_missing_quote_is_refused(conservative, market):
    at = market.end_ts - 120
    assert run(conservative, market, at=at, quote=None).reason == "no_quote"


# --------------------------------------------------------------------------- #
# impulse filter — absent entirely from v1
# --------------------------------------------------------------------------- #
def test_small_impulse_blocks_entry(conservative, market):
    at = market.end_ts - 120
    assert run(conservative, market, at=at, impulse=impulse_of(20)).reason == "impulse_too_small"


def test_impulse_at_threshold_passes(conservative, market):
    at = market.end_ts - 120
    assert run(conservative, market, at=at, impulse=impulse_of(70)).action is Action.ENTER


def test_missing_impulse_blocks_when_fail_action_is_skip(conservative, market):
    at = market.end_ts - 120
    assert run(conservative, market, at=at, impulse=None).reason == "impulse_unavailable"


def test_too_few_samples_blocks(conservative, market):
    at = market.end_ts - 120
    decision = run(conservative, market, at=at, impulse=impulse_of(120, samples=1))
    assert decision.reason == "impulse_unavailable"


def test_missing_impulse_is_ignored_when_configured(demo, market):
    """The demo profile sets fail_action: ignore."""
    at = market.end_ts - 120
    decision = evaluate(market=market, quote=make_quote(at), impulse=None,
                        profile=demo, equity_usd=100.0, at=at)
    assert decision.action is Action.ENTER


def test_never_fades_an_established_move(conservative, market):
    """Spot moved down hard, the book is bid for UP: stand aside."""
    at = market.end_ts - 120
    decision = run(conservative, market, at=at, impulse=impulse_of(-150))
    assert decision.reason == "impulse_direction_mismatch"


# --------------------------------------------------------------------------- #
# signal and microstructure
# --------------------------------------------------------------------------- #
def test_below_threshold_waits(conservative, market):
    at = market.end_ts - 120
    quote = make_quote(at, up_ask=0.55, down_ask=0.45)
    assert run(conservative, market, at=at, quote=quote).reason == "price_below_threshold"


def test_picks_the_stronger_side(conservative, market):
    at = market.end_ts - 120
    quote = make_quote(at, up_ask=0.72, down_ask=0.88)
    decision = run(conservative, market, at=at, quote=quote, impulse=impulse_of(-120))
    assert decision.side is Side.DOWN and decision.price == pytest.approx(0.88)


def test_wide_spread_is_refused(conservative, market):
    at = market.end_ts - 120
    quote = make_quote(at, spread=0.09)
    assert run(conservative, market, at=at, quote=quote).reason == "spread_too_wide"


def test_thin_book_is_refused(conservative, market):
    at = market.end_ts - 120
    quote = make_quote(at, depth=5.0)
    assert run(conservative, market, at=at, quote=quote).reason == "insufficient_liquidity"


def test_unknown_depth_is_refused_when_required(conservative, market):
    at = market.end_ts - 120
    quote = dataclasses.replace(make_quote(at), up_ask_notional=None, down_ask_notional=None)
    assert run(conservative, market, at=at, quote=quote).reason == "liquidity_unknown"


# --------------------------------------------------------------------------- #
# sizing and hedging
# --------------------------------------------------------------------------- #
def test_stake_respects_the_smallest_cap(conservative, market):
    at = market.end_ts - 120
    # 8% of $20 equity = $1.60, below both stake_usd and max_notional_usd.
    decision = run(conservative, market, at=at, equity=20.0)
    assert decision.stake_usd == pytest.approx(1.60)


def test_stake_never_exceeds_available_depth(conservative, market):
    at = market.end_ts - 120
    quote = make_quote(at, depth=32.0)   # passes the 30 guard, under the $5 stake? no - larger
    decision = run(conservative, market, at=at, quote=quote)
    assert decision.stake_usd <= 32.0

    thin = make_quote(at, depth=31.0)
    assert run(conservative, market, at=at, quote=thin).stake_usd == pytest.approx(5.0)


def test_hedge_triggers_only_on_extreme_skew_near_close(conservative, market):
    at = market.end_ts - 120
    early = market.end_ts - 140         # extreme skew, but before the hedge window
    no_hedge = run(conservative, market, at=early, quote=make_quote(early, up_ask=0.96))
    assert no_hedge.hedge_usd == 0.0

    late = market.end_ts - 100          # inside both the entry and hedge windows
    hedged = run(conservative, market, at=late, quote=make_quote(late, up_ask=0.96))
    assert hedged.action is Action.ENTER and hedged.hedge_usd == pytest.approx(1.0)


def test_zero_equity_blocks_entry(conservative, market):
    at = market.end_ts - 120
    assert run(conservative, market, at=at, equity=0.0).reason == "stake_is_zero"


# --------------------------------------------------------------------------- #
# exits
# --------------------------------------------------------------------------- #
def position_at(market, entry=0.75, stop=0.5625) -> Position:
    return Position(
        market_slug=market.slug, market_end_ts=market.end_ts, side=Side.UP,
        token_id="up", entry_price=entry, shares=6.6, cost_usdc=5.0,
        opened_at=market.end_ts - 120, stop_loss_price=stop,
    )


def test_holds_while_healthy(conservative, market):
    assert exit_reason(position=position_at(market), mark=0.80,
                       profile=conservative, at=market.end_ts - 100) is None


def test_stop_loss_fires(conservative, market):
    reason = exit_reason(position=position_at(market), mark=0.56,
                         profile=conservative, at=market.end_ts - 100)
    assert reason == "stop_loss_25pct"


def test_time_exit_fires(conservative, market):
    reason = exit_reason(position=position_at(market), mark=0.90,
                         profile=conservative, at=market.end_ts - 20)
    assert reason == "time_exit_20s_before_end"


def test_time_exit_wins_over_a_healthy_mark(conservative, market):
    """Never ride a 5m market into resolution just because it is winning."""
    reason = exit_reason(position=position_at(market), mark=0.99,
                         profile=conservative, at=market.end_ts - 5)
    assert reason is not None


def test_unknown_mark_does_not_trigger_a_stop(conservative, market):
    assert exit_reason(position=position_at(market), mark=None,
                       profile=conservative, at=market.end_ts - 100) is None
