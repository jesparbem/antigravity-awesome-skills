"""Paper execution: fills against the observed book with explicit costs.

This is what makes the project runnable and reviewable at all. It is not a
"disabled" mode — it produces real Fill objects, real PnL and real reports, so
the demo profile, the test suite and the published dashboard all exercise the
production code path.
"""

from __future__ import annotations

from typing import Optional

from ..models import Fill, Market, Side
from .base import OpenRequest


class PaperExecutor:
    """Deterministic simulated fills.

    ``slippage`` models crossing the spread on a marketable order and
    ``fee_bps`` a taker fee, so paper PnL is pessimistic rather than flattering.
    """

    name = "paper"
    live = False

    def __init__(self, *, slippage: float = 0.005, fee_bps: float = 0.0) -> None:
        self.slippage = max(0.0, float(slippage))
        self.fee_bps = max(0.0, float(fee_bps))

    def _fee(self, usdc: float) -> float:
        return usdc * (self.fee_bps / 10_000.0)

    def open(self, request: OpenRequest) -> Optional[Fill]:
        price = min(0.999, request.limit_price + self.slippage)
        if price <= 0 or request.stake_usd <= 0:
            return None
        gross = request.stake_usd
        shares = round(gross / price, 6)
        if shares <= 0:
            return None
        token = request.market.up_token if request.side is Side.UP else request.market.down_token
        return Fill(
            ts=request.at,
            side=request.side,
            token_id=token,
            price=round(price, 6),
            shares=shares,
            usdc=round(gross + self._fee(gross), 6),
            order_id=f"paper-open-{int(request.at)}",
            simulated=True,
        )

    def close(self, *, market: Market, side: Side, token_id: str, shares: float,
              mark: Optional[float], at: float) -> Optional[Fill]:
        if mark is None or shares <= 0:
            return None
        price = max(0.001, mark - self.slippage)
        gross = round(price * shares, 6)
        return Fill(
            ts=at,
            side=side,
            token_id=token_id,
            price=round(price, 6),
            shares=round(shares, 6),
            usdc=round(gross - self._fee(gross), 6),
            order_id=f"paper-close-{int(at)}",
            simulated=True,
        )
