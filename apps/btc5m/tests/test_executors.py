"""Paper fills and the live runner's output parsing."""

from __future__ import annotations

import pytest

from btc5m.executors.base import OpenRequest
from btc5m.executors.live import extract_json_objects
from btc5m.executors.paper import PaperExecutor
from btc5m.models import Side

from conftest import T0


@pytest.fixture
def paper() -> PaperExecutor:
    return PaperExecutor(slippage=0.005, fee_bps=0.0)


def test_open_crosses_the_spread(paper, market):
    fill = paper.open(OpenRequest(market=market, side=Side.UP, limit_price=0.70,
                                  stake_usd=5.0, at=T0))
    assert fill is not None
    assert fill.price == pytest.approx(0.705)      # pays slippage, never the mid
    assert fill.shares == pytest.approx(5.0 / 0.705, rel=1e-4)
    assert fill.usdc == pytest.approx(5.0)
    assert fill.simulated is True
    assert fill.token_id == market.up_token


def test_open_uses_the_down_token_for_down(paper, market):
    fill = paper.open(OpenRequest(market=market, side=Side.DOWN, limit_price=0.30,
                                  stake_usd=5.0, at=T0))
    assert fill.token_id == market.down_token


def test_zero_stake_does_not_fill(paper, market):
    assert paper.open(OpenRequest(market=market, side=Side.UP, limit_price=0.7,
                                  stake_usd=0.0, at=T0)) is None


def test_close_pays_below_the_mark(paper, market):
    fill = paper.close(market=market, side=Side.UP, token_id="up", shares=7.0,
                       mark=0.80, at=T0)
    assert fill.price == pytest.approx(0.795)
    assert fill.usdc == pytest.approx(0.795 * 7.0)


def test_close_without_a_mark_does_not_fill(paper, market):
    assert paper.close(market=market, side=Side.UP, token_id="up", shares=7.0,
                       mark=None, at=T0) is None


def test_round_trip_at_a_flat_price_loses_the_spread(paper, market):
    """Paper PnL must be pessimistic, not flattering."""
    opened = paper.open(OpenRequest(market=market, side=Side.UP, limit_price=0.70,
                                    stake_usd=5.0, at=T0))
    closed = paper.close(market=market, side=Side.UP, token_id="up",
                         shares=opened.shares, mark=0.70, at=T0 + 60)
    assert closed.usdc < opened.usdc


def test_fees_are_charged_on_both_legs(market):
    executor = PaperExecutor(slippage=0.0, fee_bps=50.0)
    opened = executor.open(OpenRequest(market=market, side=Side.UP, limit_price=0.70,
                                       stake_usd=5.0, at=T0))
    assert opened.usdc == pytest.approx(5.0 * 1.005)
    closed = executor.close(market=market, side=Side.UP, token_id="up",
                            shares=opened.shares, mark=0.70, at=T0)
    assert closed.usdc < 0.70 * opened.shares


# --------------------------------------------------------------------------- #
# live runner output parsing
# --------------------------------------------------------------------------- #
def test_extracts_multiple_objects():
    text = 'log line\n{"a": 1}\nmore\n{"order_post_result": {"success": true}}\n'
    assert extract_json_objects(text) == [{"a": 1}, {"order_post_result": {"success": True}}]


def test_braces_inside_strings_do_not_confuse_the_parser():
    """v1's brace counter dropped the order result whenever a message held a '{'."""
    text = '{"error": "unexpected { token", "order_post_result": {"success": true}}'
    parsed = extract_json_objects(text)
    assert len(parsed) == 1
    assert parsed[0]["order_post_result"]["success"] is True


def test_escaped_quotes_are_handled():
    text = r'{"msg": "he said \"hi\" {", "n": 2}'
    assert extract_json_objects(text) == [{"msg": 'he said "hi" {', "n": 2}]


def test_unterminated_object_is_ignored():
    assert extract_json_objects('noise {"a": 1') == []


def test_invalid_json_is_skipped_not_raised():
    assert extract_json_objects('{not json} {"ok": 1}') == [{"ok": 1}]


def test_live_executor_refuses_a_missing_repo(tmp_path):
    from btc5m.errors import ExecutionError
    from btc5m.executors.live import LiveSubprocessExecutor

    with pytest.raises(ExecutionError, match="trading repo not found"):
        LiveSubprocessExecutor(repo=tmp_path / "nope", live=False)


def test_live_executor_refuses_a_repo_without_the_runner(tmp_path):
    from btc5m.errors import ExecutionError
    from btc5m.executors.live import LiveSubprocessExecutor

    (tmp_path / "src" / "live").mkdir(parents=True)
    with pytest.raises(ExecutionError, match="order runner not found"):
        LiveSubprocessExecutor(repo=tmp_path, live=False)
