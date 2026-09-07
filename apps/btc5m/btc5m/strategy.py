"""Entry and exit logic — pure functions, no I/O.

Everything the bot decides happens here, over immutable inputs and an explicit
``at`` timestamp. That makes each rule directly unit-testable; v1 interleaved
these checks with HTTP calls and subprocess launches inside one 200-line loop,
so none of them could be tested at all.

Rule order is deliberate: cheap structural checks first, then time, then market
data quality, then the signal, then microstructure. The first failing rule
short-circuits and its code becomes ``Decision.reason``.
"""

from __future__ import annotations

from typing import Optional

from .config import Profile
from .models import Action, Decision, Impulse, Market, Position, Quote, Side


def evaluate(
    *,
    market: Optional[Market],
    quote: Optional[Quote],
    impulse: Optional[Impulse],
    profile: Profile,
    equity_usd: float,
    at: float,
) -> Decision:
    """Decide whether to open a position on this tick."""
    if market is None:
        return Decision(Action.WAIT, "no_active_market")

    seconds_left = market.seconds_left(at)
    if not market.tradable(at):
        return Decision(
            Action.SKIP, "market_not_tradable",
            detail={"active": market.active, "closed": market.closed, "seconds_left": round(seconds_left, 1)},
        )

    timing = profile.timing
    if seconds_left > timing.max_entry_seconds_left:
        # Too early: wait for the momentum window instead of entering at 4m left.
        return Decision(
            Action.WAIT, "too_early_to_enter",
            detail={"seconds_left": round(seconds_left, 1), "window_opens_at": timing.max_entry_seconds_left},
        )
    if seconds_left < timing.min_entry_seconds_left:
        return Decision(
            Action.SKIP, "too_late_to_enter",
            detail={"seconds_left": round(seconds_left, 1), "min_entry_seconds_left": timing.min_entry_seconds_left},
        )

    if quote is None:
        return Decision(Action.SKIP, "no_quote")

    safety = profile.safety
    age = quote.age_sec(at)
    if age > safety.skip_if_quote_stale_sec_gt:
        return Decision(
            Action.SKIP, "quote_stale",
            detail={"age_sec": round(age, 2), "max_age_sec": safety.skip_if_quote_stale_sec_gt},
        )

    # --- impulse confirmation (the documented $70-$100 move) -----------------
    imp = profile.impulse
    if imp.enabled:
        if impulse is None or impulse.samples < imp.min_samples:
            if imp.fail_action == "skip":
                return Decision(
                    Action.SKIP, "impulse_unavailable",
                    detail={"samples": impulse.samples if impulse else 0, "min_samples": imp.min_samples},
                )
        elif impulse.abs_move_usd < imp.btc_move_usd_min:
            return Decision(
                Action.SKIP, "impulse_too_small",
                detail={"move_usd": round(impulse.move_usd, 2), "min_usd": imp.btc_move_usd_min},
            )

    # --- signal --------------------------------------------------------------
    threshold = profile.signal.threshold_price
    candidates: list[tuple[Side, float]] = []
    for side in (Side.UP, Side.DOWN):
        ask = quote.ask(side)
        if ask is not None and ask >= threshold:
            candidates.append((side, float(ask)))

    if not candidates:
        return Decision(
            Action.WAIT, "price_below_threshold",
            detail={"threshold": threshold, "up_ask": quote.up_ask, "down_ask": quote.down_ask},
        )

    side, price = max(candidates, key=lambda c: c[1])

    # Follow momentum, never fade it: if spot moved one way and the book is
    # pricing the other, stand aside.
    if imp.enabled and imp.require_direction_match and impulse is not None:
        direction = impulse.direction
        if direction is not None and direction is not side:
            return Decision(
                Action.SKIP, "impulse_direction_mismatch",
                detail={"book_side": side.value, "impulse_side": direction.value,
                        "move_usd": round(impulse.move_usd, 2)},
            )

    # --- microstructure ------------------------------------------------------
    spread = quote.spread(side)
    if spread is not None and spread > safety.skip_if_spread_gt:
        return Decision(
            Action.SKIP, "spread_too_wide", side=side, price=price,
            detail={"spread": round(spread, 4), "max_spread": safety.skip_if_spread_gt},
        )

    notional = quote.ask_notional(side)
    min_notional = safety.skip_if_top_ask_notional_usd_lt
    if min_notional > 0:
        if notional is None:
            if safety.require_liquidity_data:
                return Decision(
                    Action.SKIP, "liquidity_unknown", side=side, price=price,
                    detail={"min_top_ask_notional_usd": min_notional},
                )
        elif notional < min_notional:
            return Decision(
                Action.SKIP, "insufficient_liquidity", side=side, price=price,
                detail={"top_ask_notional_usd": round(notional, 2), "min_top_ask_notional_usd": min_notional},
            )

    # --- sizing --------------------------------------------------------------
    stake = profile.sizing.stake_for(equity_usd)
    if stake <= 0:
        return Decision(Action.SKIP, "stake_is_zero", side=side, price=price,
                        detail={"equity_usd": round(equity_usd, 2)})

    # Never buy more notional than the book can actually fill at the top level.
    if notional is not None and notional < stake:
        stake = round(notional, 4)

    hedge = profile.hedge.size_for(stake, price, seconds_left)

    return Decision(
        Action.ENTER, "signal_confirmed", side=side, price=price,
        stake_usd=stake, hedge_usd=hedge,
        detail={
            "seconds_left": round(seconds_left, 1),
            "threshold": threshold,
            "spread": round(spread, 4) if spread is not None else None,
            "top_ask_notional_usd": round(notional, 2) if notional is not None else None,
            "impulse_move_usd": round(impulse.move_usd, 2) if impulse else None,
        },
    )


def exit_reason(
    *,
    position: Position,
    mark: Optional[float],
    profile: Profile,
    at: float,
) -> Optional[str]:
    """Return a close reason, or ``None`` to keep holding."""
    seconds_left = position.market_end_ts - at
    if seconds_left <= profile.timing.exit_before_sec:
        return f"time_exit_{profile.timing.exit_before_sec}s_before_end"
    if profile.stop_loss.enabled and mark is not None and mark <= position.stop_loss_price:
        pct = int(profile.stop_loss.stop_loss_pct_from_entry * 100)
        return f"stop_loss_{pct}pct"
    return None
