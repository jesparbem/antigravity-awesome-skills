"""``btc5m`` command line.

Subcommands
-----------
``tui``       live terminal dashboard
``run``       headless session, JSON on stdout
``report``    PnL report over the trade store
``profiles``  show resolved profile parameters
``doctor``    preflight checks before a live session

Live trading is opt-in twice: ``--execute`` *and* an interactive confirmation
(or ``--yes``). v1 needed only ``--execute``, one word away from a dry run.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from typing import Optional, Sequence

from . import __version__
from .config import load_profiles
from .errors import Btc5mError
from .factory import DEFAULT_DB, build_session
from .logging_setup import configure
from .models import now_ts
from .reporting import build_report
from .store import Store


def _add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--profile", default="conservative", help="profile name (default: conservative)")
    parser.add_argument("--config", default=None, help="path to btc_5m_profiles.yaml")
    parser.add_argument("--source", default="simulated", choices=("simulated", "polymarket"),
                        help="market data source (default: simulated)")
    parser.add_argument("--db", default=str(DEFAULT_DB), help="SQLite trade store path")
    parser.add_argument("--equity", type=float, default=100.0, help="starting equity in USDC")
    parser.add_argument("--execute", action="store_true",
                        help="place REAL orders through the trading repo (default: paper)")
    parser.add_argument("--repo", default=None, help="trading repo path (implies BTC5M_REPO)")
    parser.add_argument("--yes", action="store_true", help="skip the live-trading confirmation prompt")
    parser.add_argument("--max-ticks", type=int, default=None, help="stop after N polls")
    parser.add_argument("--log-level", default="INFO")
    parser.add_argument("--json-logs", action="store_true")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="btc5m", description="BTC 5m Up/Down trading on Polymarket")
    parser.add_argument("--version", action="version", version=f"btc5m {__version__}")
    sub = parser.add_subparsers(dest="command", required=True)

    tui = sub.add_parser("tui", help="live terminal dashboard")
    _add_common(tui)

    run = sub.add_parser("run", help="headless session, JSON snapshot on stdout")
    _add_common(run)

    report = sub.add_parser("report", help="PnL report from the trade store")
    report.add_argument("--db", default=str(DEFAULT_DB))
    report.add_argument("--limit", type=int, default=25)

    profiles = sub.add_parser("profiles", help="show resolved profiles")
    profiles.add_argument("--config", default=None)
    profiles.add_argument("--profile", default=None, help="show just this one")

    doctor = sub.add_parser("doctor", help="preflight checks")
    doctor.add_argument("--config", default=None)
    doctor.add_argument("--source", default="simulated", choices=("simulated", "polymarket"))
    doctor.add_argument("--repo", default=None)
    return parser


def _confirm_live(profile_name: str, assume_yes: bool) -> bool:
    if profile_name == "demo":
        print("refusing: the 'demo' profile is paper-only. Pick another profile for live trading.",
              file=sys.stderr)
        return False
    if assume_yes:
        return True
    if not sys.stdin.isatty():
        print("refusing: --execute needs a terminal for confirmation, or pass --yes.", file=sys.stderr)
        return False
    print(f"\n  LIVE TRADING with profile '{profile_name}'. Real orders, real money.")
    return input("  Type 'yes' to continue: ").strip().lower() == "yes"


def _session_from_args(args: argparse.Namespace):
    profiles = load_profiles(args.config)
    profile = profiles.get(args.profile)
    live = bool(args.execute)
    if live and not _confirm_live(profile.name, args.yes):
        raise SystemExit(2)
    if live and args.source != "polymarket":
        print("note: --execute forces --source polymarket", file=sys.stderr)
        args.source = "polymarket"
    return build_session(
        profile=profile, source_name=args.source, live=live,
        repo=args.repo, equity_usd=args.equity, db_path=args.db,
    )


def cmd_tui(args: argparse.Namespace) -> int:
    from . import tui  # rich import kept out of the non-TUI paths

    session, store = _session_from_args(args)
    deadline = time.time() + session.profile.runner.entry_timeout_min * 60
    try:
        tui.run(session, max_ticks=args.max_ticks, deadline_ts=deadline)
    finally:
        store.close()
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    session, store = _session_from_args(args)
    deadline = time.time() + session.profile.runner.entry_timeout_min * 60
    try:
        snapshot = session.run(max_ticks=args.max_ticks, deadline_ts=deadline)
        print(json.dumps(snapshot.as_dict(), ensure_ascii=False, indent=2))
    finally:
        store.close()
    return 0


def cmd_report(args: argparse.Namespace) -> int:
    store = Store(args.db)
    try:
        print(json.dumps(build_report(store, at=now_ts(), limit=args.limit),
                         ensure_ascii=False, indent=2, default=str))
    finally:
        store.close()
    return 0


def cmd_profiles(args: argparse.Namespace) -> int:
    profiles = load_profiles(args.config)
    if args.profile:
        print(json.dumps(profiles.get(args.profile).as_dict(), ensure_ascii=False, indent=2))
    else:
        print(json.dumps({name: profiles.get(name).as_dict() for name in profiles.names},
                         ensure_ascii=False, indent=2))
    return 0


def cmd_doctor(args: argparse.Namespace) -> int:
    checks: list[tuple[str, bool, str]] = []

    try:
        profiles = load_profiles(args.config)
        checks.append(("config", True, f"{len(profiles.names)} profiles: {', '.join(profiles.names)}"))
    except Btc5mError as exc:
        checks.append(("config", False, str(exc)))
        profiles = None

    if args.source == "polymarket":
        from .sources.polymarket import PolymarketSource

        try:
            with PolymarketSource() as source:
                market = source.current_market(time.time())
            checks.append(("polymarket", market is not None,
                           market.slug if market else "no tradable slot resolved"))
        except Btc5mError as exc:
            checks.append(("polymarket", False, str(exc)))
    else:
        checks.append(("polymarket", True, "skipped (simulated source)"))

    if args.repo:
        try:
            from .executors.live import LiveSubprocessExecutor

            LiveSubprocessExecutor(repo=args.repo, live=False)
            checks.append(("trading repo", True, args.repo))
        except Btc5mError as exc:
            checks.append(("trading repo", False, str(exc)))

    width = max(len(name) for name, _, _ in checks)
    for name, ok, detail in checks:
        print(f"{'PASS' if ok else 'FAIL'}  {name.ljust(width)}  {detail}")
    return 0 if all(ok for _, ok, _ in checks) else 1


COMMANDS = {
    "tui": cmd_tui, "run": cmd_run, "report": cmd_report,
    "profiles": cmd_profiles, "doctor": cmd_doctor,
}


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    configure(getattr(args, "log_level", "INFO"), json_logs=getattr(args, "json_logs", False))
    try:
        return COMMANDS[args.command](args)
    except Btc5mError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
