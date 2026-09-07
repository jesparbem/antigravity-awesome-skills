#!/usr/bin/env python3
"""Render the terminal dashboard to SVG/HTML for documentation.

Runs the simulated market forward until an interesting state is reached (an open
position, ideally with a hedge), then captures that frame. No TTY required.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from rich.console import Console

from btc5m import tui
from btc5m.config import load_profiles
from btc5m.engine import Session
from btc5m.executors.paper import PaperExecutor
from btc5m.models import bucket_5m
from btc5m.risk import RiskState
from btc5m.sources.price import SimulatedPriceFeed
from btc5m.sources.simulated import SimulatedSource
from btc5m.store import Store


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="docs/tui.svg")
    parser.add_argument("--profile", default="demo")
    parser.add_argument("--width", type=int, default=110)
    parser.add_argument("--height", type=int, default=30)
    parser.add_argument("--start", type=int, default=1788800100, help="UTC seconds to start the simulation at")
    parser.add_argument("--max-ticks", type=int, default=4000)
    args = parser.parse_args()

    profile = load_profiles().get(args.profile)
    clock_value = [float(bucket_5m(args.start))]
    store = Store(Path(args.out).with_suffix(".capture.db"))
    session = Session(
        profile=profile, source=SimulatedSource(), price_feed=SimulatedPriceFeed(),
        executor=PaperExecutor(), risk=RiskState(profile=profile, starting_equity_usd=100.0),
        store=store, clock=lambda: clock_value[0],
    )

    best = None
    for _ in range(args.max_ticks):
        snapshot = session.tick()
        clock_value[0] += 2.0
        if snapshot.position is not None and snapshot.risk.get("trades_today", 0) >= 2:
            best = snapshot
            if snapshot.position.hedge is not None:
                break
    snapshot = best or session.snapshot

    console = Console(record=True, width=args.width, height=args.height, force_terminal=True)
    console.print(tui.render(snapshot, "simulated"))
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if out.suffix == ".html":
        console.save_html(str(out), inline_styles=True)
    else:
        console.save_svg(str(out), title="btc5m — terminal dashboard")
    store.close()
    Path(str(out.with_suffix(".capture.db"))).unlink(missing_ok=True)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
