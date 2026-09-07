"""Session assembly shared by the terminal and web front-ends.

Both entry points build a session the same way, so a profile behaves
identically whichever UI is driving it.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Optional

from .config import Profile, ProfileSet, load_profiles
from .engine import Session
from .errors import ConfigError
from .executors.base import ExecutionEngine
from .executors.paper import PaperExecutor
from .models import now_ts
from .reporting import day_start_ts
from .risk import RiskState
from .sources.price import SimulatedPriceFeed, build_price_feed
from .sources.polymarket import PolymarketSource
from .sources.simulated import SimulatedSource
from .store import Store

DEFAULT_DB = Path(os.environ.get("BTC5M_DB", "runtime/btc5m.db"))


def build_source(name: str) -> Any:
    if name == "polymarket":
        return PolymarketSource()
    if name == "simulated":
        return SimulatedSource()
    raise ConfigError(f"unknown source {name!r}; expected 'polymarket' or 'simulated'")


def build_executor(*, live: bool, repo: Optional[str] = None,
                   slippage: float = 0.005) -> ExecutionEngine:
    """Paper unless ``live`` is explicitly requested."""
    if not live:
        return PaperExecutor(slippage=slippage)
    from .executors.live import LiveSubprocessExecutor  # imported lazily: needs a real repo

    resolved = repo or os.environ.get("BTC5M_REPO")
    if not resolved:
        raise ConfigError(
            "live execution requires the trading repo path: pass --repo or set BTC5M_REPO"
        )
    return LiveSubprocessExecutor(
        repo=resolved,
        python_bin=os.environ.get("BTC5M_PYTHON"),
        live=True,
    )


def build_session(
    *,
    profile: Profile,
    source_name: str = "simulated",
    live: bool = False,
    repo: Optional[str] = None,
    equity_usd: float = 100.0,
    db_path: Optional[str | Path] = None,
) -> tuple[Session, Store]:
    source = build_source(source_name)
    if source_name == "simulated":
        price_feed: Any = SimulatedPriceFeed()
    else:
        price_feed = build_price_feed(profile.impulse.source, profile.impulse.symbol)

    store = Store(Path(db_path) if db_path else DEFAULT_DB)

    # Carry today's realised result forward so the daily caps survive a restart.
    risk = RiskState(profile=profile, starting_equity_usd=equity_usd)
    today = store.stats(since_ts=day_start_ts(now_ts()))
    risk.seed(realised_pnl_usdc=today["pnl_usdc"], trades=today["trades"])

    session = Session(
        profile=profile,
        source=source,
        price_feed=price_feed,
        executor=build_executor(live=live, repo=repo),
        risk=risk,
        store=store,
    )
    return session, store


def load(config_path: Optional[str] = None) -> ProfileSet:
    return load_profiles(config_path)
