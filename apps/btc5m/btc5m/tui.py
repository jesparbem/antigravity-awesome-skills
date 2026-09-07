"""Terminal dashboard.

A live console view of the same :class:`~btc5m.engine.Snapshot` the web UI
renders, so the two front-ends can never drift apart. Rendering is a pure
function of a snapshot, which means the layout is testable and can be captured
to SVG for documentation without a TTY.
"""

from __future__ import annotations

import time
from typing import Optional

from rich.align import Align
from rich.console import Console, Group, RenderableType
from rich.layout import Layout
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from .engine import Session, Snapshot
from .models import Action, Side, iso

ACCENT = "bright_cyan"

REASON_STYLE = {
    "signal_confirmed": "bold green",
    "too_early_to_enter": "dim",
    "price_below_threshold": "dim",
    "no_active_market": "dim",
    "kill_switch": "bold red",
    "daily_loss_limit": "bold red",
    "max_trades_per_day": "bold red",
    "source_circuit_breaker": "bold red",
}

REASON_TEXT = {
    "signal_confirmed": "signal confirmed - entering",
    "holding_position": "holding - watching stop and clock",
    "closed": "position closed",
    "too_early_to_enter": "waiting for the entry window",
    "too_late_to_enter": "past the entry window",
    "price_below_threshold": "no side above the price threshold",
    "impulse_too_small": "BTC move below the impulse minimum",
    "impulse_unavailable": "spot feed unavailable",
    "impulse_direction_mismatch": "book disagrees with the move - not fading it",
    "spread_too_wide": "spread above the guard",
    "insufficient_liquidity": "top of book too thin",
    "liquidity_unknown": "book depth unknown",
    "quote_stale": "quote too old",
    "market_not_tradable": "market closed or inactive",
    "no_active_market": "no active 5m slot",
    "no_quote": "no order book",
    "stake_is_zero": "sizing resolved to zero",
    "kill_switch": "KILL SWITCH ENGAGED",
    "daily_loss_limit": "daily loss cap reached",
    "max_trades_per_day": "daily trade cap reached",
    "source_circuit_breaker": "market data circuit breaker open",
}


def _bar(fraction: float, width: int = 22, *, style: str = ACCENT) -> Text:
    fraction = max(0.0, min(1.0, fraction))
    filled = int(round(fraction * width))
    return Text("█" * filled, style=style) + Text("░" * (width - filled), style="grey37")


def _money(value: Optional[float], suffix: str = "") -> Text:
    if value is None:
        return Text("--", style="grey50")
    style = "green" if value > 0 else "red" if value < 0 else "white"
    return Text(f"{value:+,.4f}{suffix}", style=style)


def header(snap: Snapshot, source_name: str) -> RenderableType:
    mode = Text(f" {snap.mode.upper()} ", style="bold white on red" if snap.mode == "live"
                else "bold black on bright_green")
    state = Text(" RUNNING " if snap.running else " STOPPED ",
                 style="bold black on bright_green" if snap.running else "bold white on grey30")
    line = Text.assemble(
        ("btc5m", f"bold {ACCENT}"), ("  profile=", "grey62"), (snap.profile, "bold white"),
        ("  source=", "grey62"), (source_name, "white"),
        ("  tick=", "grey62"), (str(snap.ticks), "white"),
        ("  ", ""),
    )
    return Panel(Align.left(Group(Text.assemble(line, mode, Text("  "), state))),
                 border_style=ACCENT, padding=(0, 1))


def market_panel(snap: Snapshot) -> RenderableType:
    if snap.market is None:
        return Panel(Align.center(Text("no active 5m slot", style="grey50")),
                     title="market", border_style="grey37")
    left = snap.market.seconds_left(snap.ts)
    elapsed = max(0.0, min(1.0, 1.0 - left / 300.0))
    urgency = "red" if left < 30 else "yellow" if left < 90 else ACCENT

    table = Table.grid(padding=(0, 1))
    table.add_column(style="grey62", justify="right")
    table.add_column()
    table.add_row("slug", Text(snap.market.slug, style="white"))
    table.add_row("closes", Text(iso(snap.market.end_ts), style="white"))
    table.add_row("time left", Text.assemble((f"{left:6.1f}s  ", f"bold {urgency}"), _bar(elapsed, style=urgency)))

    if snap.impulse is not None:
        move = snap.impulse.move_usd
        arrow = "▲" if move > 0 else "▼" if move < 0 else "="
        table.add_row("BTC spot", Text(f"${snap.spot:,.2f}" if snap.spot else "--", style="white"))
        table.add_row("in-slot move",
                      Text(f"{arrow} ${move:+,.2f}", style="green" if move > 0 else "red" if move < 0 else "white"))
    return Panel(table, title="market", border_style=ACCENT, padding=(0, 1))


def book_panel(snap: Snapshot) -> RenderableType:
    table = Table(expand=True, box=None, pad_edge=False)
    table.add_column("side", style="grey62")
    table.add_column("bid", justify="right")
    table.add_column("ask", justify="right")
    table.add_column("spread", justify="right")
    table.add_column("ask depth", justify="right")

    quote = snap.quote
    if quote is None:
        return Panel(Align.center(Text("no order book", style="grey50")),
                     title="order book", border_style="grey37")

    for side in (Side.UP, Side.DOWN):
        ask = quote.ask(side)
        bid = quote.bid(side)
        spread = quote.spread(side)
        depth = quote.ask_notional(side)
        highlight = "bold green" if ask is not None and ask >= 0.7 else "white"
        table.add_row(
            Text(side.value, style="bold"),
            Text(f"{bid:.3f}" if bid is not None else "--", style="white"),
            Text(f"{ask:.3f}" if ask is not None else "--", style=highlight),
            Text(f"{spread:.3f}" if spread is not None else "--", style="white"),
            Text(f"${depth:,.0f}" if depth is not None else "--", style="white"),
        )
    return Panel(table, title="order book", border_style=ACCENT, padding=(0, 1))


def decision_panel(snap: Snapshot) -> RenderableType:
    decision = snap.decision
    if decision is None:
        return Panel(Align.center(Text("evaluating...", style="grey50")),
                     title="decision", border_style="grey37")

    reason = decision.reason
    style = REASON_STYLE.get(reason, "yellow" if decision.action is Action.SKIP else "white")
    icon = {"enter": "●", "wait": "◌", "skip": "○"}.get(decision.action.value, "·")

    body = Table.grid(padding=(0, 1))
    body.add_column(style="grey62", justify="right")
    body.add_column()
    body.add_row("action", Text(f"{icon} {decision.action.value.upper()}", style=style))
    body.add_row("reason", Text(REASON_TEXT.get(reason, reason), style=style))
    if decision.side:
        body.add_row("side", Text(decision.side.value, style="bold"))
    if decision.price is not None:
        body.add_row("price", Text(f"{decision.price:.3f}"))
    if decision.entering:
        body.add_row("stake", Text(f"${decision.stake_usd:,.2f}", style="bold"))
        if decision.hedge_usd:
            body.add_row("hedge", Text(f"${decision.hedge_usd:,.2f}", style="magenta"))
    for key, value in list(decision.detail.items())[:4]:
        body.add_row(key.replace("_", " "), Text(str(value), style="grey70"))
    return Panel(body, title="decision", border_style=style if decision.entering else "grey37",
                 padding=(0, 1))


def position_panel(snap: Snapshot) -> RenderableType:
    position = snap.position
    if position is None:
        return Panel(Align.center(Text("flat", style="grey50")), title="position",
                     border_style="grey37", padding=(0, 1))
    unrealised = position.unrealised(snap.mark)
    table = Table.grid(padding=(0, 1))
    table.add_column(style="grey62", justify="right")
    table.add_column()
    # Ordered by what a trader checks first; the panel is height-bounded, so the
    # decisive numbers must come before the bookkeeping ones.
    table.add_row("side", Text(position.side.value, style="bold"))
    table.add_row("unrealised", _money(unrealised, " USDC"))
    table.add_row("entry -> mark", Text.assemble(
        (f"{position.entry_price:.3f}", "white"), (" -> ", "grey62"),
        (f"{snap.mark:.3f}" if snap.mark is not None else "--", "bold white"),
    ))
    table.add_row("stop", Text(f"{position.stop_loss_price:.3f}", style="red"))
    table.add_row("size", Text(f"{position.shares:,.4f} sh / ${position.cost_usdc:,.2f}"))
    if position.hedge is not None:
        table.add_row("hedge", Text(f"{position.hedge.side.value} ${position.hedge.usdc:,.2f}", style="magenta"))
    return Panel(table, title="position", border_style="green", padding=(0, 1))


def risk_panel(snap: Snapshot) -> RenderableType:
    risk = snap.risk or {}
    table = Table.grid(padding=(0, 1))
    table.add_column(style="grey62", justify="right")
    table.add_column()
    table.add_row("equity", Text(f"${risk.get('equity_usd', 0):,.2f}", style="bold"))
    table.add_row("realised", _money(risk.get("realised_pnl_usdc"), " USDC"))

    trades = risk.get("trades_today", 0)
    cap = risk.get("max_trades_per_day", 1) or 1
    table.add_row("trades today", Text.assemble((f"{trades}/{cap} ", "white"), _bar(trades / cap, 14)))

    used = risk.get("daily_loss_used_pct", 0.0)
    loss_style = "red" if used >= 80 else "yellow" if used >= 50 else "green"
    table.add_row("daily loss", Text.assemble((f"{used:5.1f}% ", loss_style), _bar(used / 100, 14, style=loss_style)))

    if risk.get("kill_switch"):
        table.add_row("kill switch", Text(f"ENGAGED - {risk.get('kill_switch_reason', '')}", style="bold white on red"))
    if snap.block_reason:
        table.add_row("blocked", Text(REASON_TEXT.get(snap.block_reason, snap.block_reason), style="bold red"))
    return Panel(table, title="risk", border_style="grey37", padding=(0, 1))


def footer(snap: Snapshot) -> RenderableType:
    parts: list[RenderableType] = []
    trade = snap.last_trade
    if trade is not None:
        parts.append(Text.assemble(
            ("last trade  ", "grey62"),
            (f"{trade.side.value} {trade.market_slug} ", "white"),
            (f"{trade.pnl_usdc:+.4f} USDC", "green" if trade.pnl_usdc >= 0 else "red"),
            (f"  ({trade.close_reason})", "grey62"),
        ))
    if snap.last_error:
        parts.append(Text(f"last error  {snap.last_error[:110]}", style="red"))
    if not parts:
        parts.append(Text("no trades yet", style="grey50"))
    return Panel(Group(*parts), border_style="grey37", padding=(0, 1))


def render(snap: Snapshot, source_name: str = "-") -> Layout:
    """Compose the full dashboard for one snapshot."""
    layout = Layout()
    layout.split_column(
        Layout(header(snap, source_name), name="header", size=3),
        Layout(name="body"),
        Layout(footer(snap), name="footer", size=4),
    )
    layout["body"].split_row(Layout(name="left"), Layout(name="right"))
    layout["body"]["left"].split_column(
        Layout(market_panel(snap), name="market", size=8),
        Layout(book_panel(snap), name="book"),
    )
    layout["body"]["right"].split_column(
        Layout(decision_panel(snap), name="decision", ratio=2),
        Layout(position_panel(snap), name="position", ratio=3),
        Layout(risk_panel(snap), name="risk", size=8),
    )
    return layout


def run(session: Session, *, console: Optional[Console] = None,
        max_ticks: Optional[int] = None, deadline_ts: Optional[float] = None) -> Snapshot:
    """Drive the session, repainting after every tick. Ctrl-C exits cleanly."""
    console = console or Console()
    source_name = getattr(session.source, "name", "-")
    poll = session.profile.runner.poll_sec
    ticks = 0
    snap = session.snapshot

    with Live(render(snap, source_name), console=console, refresh_per_second=4,
              screen=True, transient=False) as live:
        try:
            while True:
                snap = session.tick()
                live.update(render(snap, source_name))
                ticks += 1
                if max_ticks is not None and ticks >= max_ticks:
                    break
                if deadline_ts is not None and time.time() >= deadline_ts and session.position is None:
                    break
                time.sleep(poll)
        except KeyboardInterrupt:
            console.print("\n[yellow]interrupted - closing cleanly[/yellow]")
    return snap
