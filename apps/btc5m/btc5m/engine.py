"""The trading session: one state machine, driven by both front-ends.

v1 was a single ``main()`` that opened a position, then entered a second
unbounded ``while True`` to babysit it, printing one JSON blob at the very end.
Nothing could observe it while it ran, and stopping it meant SIGKILL.

Here a session is an object with a ``tick()`` that advances exactly one step and
publishes an immutable :class:`Snapshot`. The TUI renders snapshots, the web API
serialises them, and the tests call ``tick()`` directly with a frozen clock.
"""

from __future__ import annotations

import logging
import threading
import time
from dataclasses import dataclass, field
from typing import Any, Callable, Optional

from .config import Profile
from .errors import TransientSourceError
from .executors.base import ExecutionEngine, OpenRequest
from .models import (
    Action, Decision, Fill, Impulse, Market, Position, Quote, Side, TradeResult, iso,
)
from .risk import RiskState
from .store import Store
from . import strategy

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Snapshot:
    """Everything a UI needs to render one moment of the session."""

    ts: float
    profile: str
    mode: str
    running: bool
    market: Optional[Market] = None
    quote: Optional[Quote] = None
    impulse: Optional[Impulse] = None
    spot: Optional[float] = None
    decision: Optional[Decision] = None
    position: Optional[Position] = None
    mark: Optional[float] = None
    block_reason: Optional[str] = None
    risk: dict[str, Any] = field(default_factory=dict)
    last_trade: Optional[TradeResult] = None
    last_error: Optional[str] = None
    ticks: int = 0

    def as_dict(self) -> dict[str, Any]:
        return {
            "ts": self.ts,
            "ts_iso": iso(self.ts),
            "profile": self.profile,
            "mode": self.mode,
            "running": self.running,
            "ticks": self.ticks,
            "seconds_left": round(self.market.seconds_left(self.ts), 1) if self.market else None,
            "market": self.market.as_dict() if self.market else None,
            "quote": self.quote.as_dict() if self.quote else None,
            "impulse": self.impulse.as_dict() if self.impulse else None,
            "spot": self.spot,
            "decision": self.decision.as_dict() if self.decision else None,
            "position": self.position.as_dict() if self.position else None,
            "mark": self.mark,
            "unrealised_usdc": self.position.unrealised(self.mark) if self.position else None,
            "block_reason": self.block_reason,
            "risk": self.risk,
            "last_trade": self.last_trade.as_dict() if self.last_trade else None,
            "last_error": self.last_error,
        }


class Session:
    """Owns the position lifecycle for one profile."""

    def __init__(
        self,
        *,
        profile: Profile,
        source: Any,
        price_feed: Optional[Any],
        executor: ExecutionEngine,
        risk: RiskState,
        store: Optional[Store] = None,
        clock: Callable[[], float] = time.time,
    ) -> None:
        self.profile = profile
        self.source = source
        self.price_feed = price_feed
        self.executor = executor
        self.risk = risk
        self.store = store
        self.clock = clock

        self.position: Optional[Position] = None
        self._ticks = 0
        self._running = False
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._snapshot = Snapshot(
            ts=clock(), profile=profile.name,
            mode="live" if getattr(executor, "live", False) else "paper",
            running=False, risk=risk.as_dict(),
        )

    # -- observation --------------------------------------------------------- #
    @property
    def snapshot(self) -> Snapshot:
        with self._lock:
            return self._snapshot

    @property
    def mode(self) -> str:
        return "live" if getattr(self.executor, "live", False) else "paper"

    def _publish(self, **kwargs: Any) -> Snapshot:
        # The most recent completed trade stays visible on every snapshot, not
        # just the tick that closed it.
        kwargs.setdefault("last_trade", self._last_trade)
        snap = Snapshot(
            ts=kwargs.pop("ts"),
            profile=self.profile.name,
            mode=self.mode,
            running=self._running,
            ticks=self._ticks,
            risk=self.risk.as_dict(),
            position=self.position,
            **kwargs,
        )
        with self._lock:
            self._snapshot = snap
        return snap

    def _event(self, ts: float, kind: str, payload: dict[str, Any]) -> None:
        if self.store is not None:
            self.store.record_event(ts, kind, payload)

    # -- one step ------------------------------------------------------------ #
    def tick(self) -> Snapshot:
        """Advance the state machine by one step and publish a snapshot."""
        at = self.clock()
        self._ticks += 1
        self.risk.roll_day(at)

        spot: Optional[float] = None
        impulse: Optional[Impulse] = None
        last_error: Optional[str] = None

        if self.price_feed is not None:
            try:
                spot = self.price_feed.sample(at)
                impulse = self.price_feed.impulse(at)
            except TransientSourceError as exc:
                last_error = str(exc)
                log.debug("price feed unavailable: %s", exc)

        try:
            market = self.source.current_market(at)
            quote = self.source.quote(market, at) if market else None
            self.risk.record_source_ok()
        except TransientSourceError as exc:
            count = self.risk.record_source_error()
            last_error = str(exc)
            log.warning("market source error %d: %s", count, exc)
            return self._publish(ts=at, spot=spot, impulse=impulse, last_error=last_error,
                                 block_reason=self.risk.block_reason(at))

        if self.position is not None:
            return self._manage_position(at, market, quote, spot, impulse, last_error)

        block = self.risk.block_reason(at)
        if block:
            decision = Decision(Action.SKIP, block)
            self._event(at, "blocked", {"reason": block})
            return self._publish(ts=at, market=market, quote=quote, impulse=impulse, spot=spot,
                                 decision=decision, block_reason=block, last_error=last_error)

        decision = strategy.evaluate(
            market=market, quote=quote, impulse=impulse, profile=self.profile,
            equity_usd=self.risk.equity_usd, at=at,
        )
        if not decision.entering:
            return self._publish(ts=at, market=market, quote=quote, impulse=impulse, spot=spot,
                                 decision=decision, last_error=last_error)

        self._open(at, market, decision)  # market is non-None whenever ENTER is returned
        return self._publish(ts=at, market=market, quote=quote, impulse=impulse, spot=spot,
                             decision=decision, last_error=last_error)

    # -- transitions --------------------------------------------------------- #
    def _open(self, at: float, market: Market, decision: Decision) -> None:
        assert decision.side is not None and decision.price is not None
        request = OpenRequest(
            market=market, side=decision.side,
            limit_price=decision.price, stake_usd=decision.stake_usd, at=at,
        )
        try:
            fill = self.executor.open(request)
        except Exception as exc:  # executor failures must not kill the session
            log.error("open failed: %s", exc)
            self._event(at, "open_failed", {"error": str(exc)})
            self.risk.record_source_error()
            return

        if fill is None:
            self._event(at, "open_unfilled", {"side": decision.side.value, "price": decision.price})
            return

        hedge_fill: Optional[Fill] = None
        if decision.hedge_usd > 0:
            hedge_fill = self._open_hedge(at, market, decision)

        self.position = Position(
            market_slug=market.slug,
            market_end_ts=market.end_ts,
            side=fill.side,
            token_id=fill.token_id,
            entry_price=fill.price,
            shares=fill.shares,
            cost_usdc=fill.usdc,
            opened_at=at,
            stop_loss_price=self.profile.stop_loss.price_for(fill.price),
            hedge=hedge_fill,
            open_fill=fill,
        )
        self._event(at, "opened", self.position.as_dict())
        log.info("opened %s %s @ %.3f (%.4f shares)", fill.side.value, market.slug, fill.price, fill.shares)

    def _open_hedge(self, at: float, market: Market, decision: Decision) -> Optional[Fill]:
        """Small opposite position, per the extreme-skew rule."""
        assert decision.side is not None
        opposite = decision.side.opposite
        price = max(0.01, 1.0 - (decision.price or 0.5))
        try:
            return self.executor.open(OpenRequest(
                market=market, side=opposite, limit_price=price,
                stake_usd=decision.hedge_usd, at=at,
            ))
        except Exception as exc:
            log.warning("hedge leg failed (main position kept): %s", exc)
            self._event(at, "hedge_failed", {"error": str(exc)})
            return None

    def _manage_position(
        self, at: float, market: Optional[Market], quote: Optional[Quote],
        spot: Optional[float], impulse: Optional[Impulse], last_error: Optional[str],
    ) -> Snapshot:
        position = self.position
        assert position is not None

        mark: Optional[float] = None
        if market is not None and market.slug == position.market_slug:
            try:
                mark = self.source.mark_price(market, position.side, at)
            except TransientSourceError as exc:
                last_error = str(exc)
        if mark is None and quote is not None:
            mark = quote.bid(position.side)

        reason = strategy.exit_reason(position=position, mark=mark, profile=self.profile, at=at)
        if reason is None:
            # Keep the UI informative while holding: show what we are waiting for.
            holding = Decision(
                Action.WAIT, "holding_position",
                side=position.side, price=mark,
                detail={
                    "stop_loss_price": position.stop_loss_price,
                    "exit_at_seconds_left": self.profile.timing.exit_before_sec,
                    "seconds_to_exit": round(
                        position.market_end_ts - self.profile.timing.exit_before_sec - at, 1
                    ),
                },
            )
            return self._publish(ts=at, market=market, quote=quote, impulse=impulse, spot=spot,
                                 mark=mark, decision=holding, last_error=last_error)

        self._close(at, market, position, mark, reason)
        return self._publish(ts=at, market=market, quote=quote, impulse=impulse, spot=spot,
                             mark=mark, last_error=last_error,
                             decision=Decision(Action.SKIP, "closed", side=position.side, price=mark,
                                               detail={"close_reason": reason}))

    _last_trade: Optional[TradeResult] = None

    def _close(self, at: float, market: Optional[Market], position: Position,
               mark: Optional[float], reason: str) -> None:
        target = market if (market and market.slug == position.market_slug) else Market(
            slug=position.market_slug, up_token=position.token_id, down_token=position.token_id,
            end_ts=position.market_end_ts,
        )
        proceeds = 0.0
        exit_price: Optional[float] = None
        try:
            fill = self.executor.close(
                market=target, side=position.side, token_id=position.token_id,
                shares=position.shares, mark=mark, at=at,
            )
        except Exception as exc:
            log.error("close failed: %s", exc)
            self._event(at, "close_failed", {"error": str(exc), "reason": reason})
            return

        if fill is None:
            # Keep holding and retry next tick rather than losing track of the
            # position, which is what v1's bounded retry loop did on give-up.
            self._event(at, "close_unfilled", {"reason": reason, "mark": mark})
            return

        proceeds += fill.usdc
        exit_price = fill.price

        hedge_cost = 0.0
        if position.hedge is not None:
            hedge_cost = position.hedge.usdc
            hedge_mark = 1.0 - (mark or position.entry_price)
            try:
                hedge_close = self.executor.close(
                    market=target, side=position.hedge.side, token_id=position.hedge.token_id,
                    shares=position.hedge.shares, mark=max(0.001, min(0.999, hedge_mark)), at=at,
                )
                if hedge_close is not None:
                    proceeds += hedge_close.usdc
            except Exception as exc:
                log.warning("hedge close failed: %s", exc)

        result = TradeResult(
            market_slug=position.market_slug, side=position.side, profile=self.profile.name,
            opened_at=position.opened_at, closed_at=at,
            entry_price=position.entry_price, exit_price=exit_price,
            shares=position.shares, cost_usdc=position.cost_usdc,
            proceeds_usdc=round(proceeds, 6), close_reason=reason,
            simulated=not getattr(self.executor, "live", False), hedge_usdc=hedge_cost,
        )
        self.risk.record_trade(result)
        if self.store is not None:
            self.store.record_trade(result)
        self._event(at, "closed", result.as_dict())
        self._last_trade = result
        self.position = None
        log.info("closed %s %s: pnl %+.4f USDC (%s)",
                 result.side.value, result.market_slug, result.pnl_usdc, reason)

    # -- loop ---------------------------------------------------------------- #
    def request_stop(self) -> None:
        self._stop.set()

    def run(self, *, max_ticks: Optional[int] = None, deadline_ts: Optional[float] = None) -> Snapshot:
        """Poll until stopped, out of ticks, or past the entry timeout.

        An open position always finishes: the deadline stops new entries, it
        never abandons a live position.
        """
        self._stop.clear()
        self._running = True
        poll = self.profile.runner.poll_sec
        ticks = 0
        try:
            while not self._stop.is_set():
                snap = self.tick()
                ticks += 1
                if max_ticks is not None and ticks >= max_ticks:
                    break
                if deadline_ts is not None and self.clock() >= deadline_ts and self.position is None:
                    break
                self._stop.wait(poll)
        finally:
            self._running = False
            last = self.snapshot
            self._publish(ts=self.clock(), market=last.market, quote=last.quote,
                          impulse=last.impulse, spot=last.spot,
                          decision=last.decision, mark=last.mark)
        return self.snapshot
