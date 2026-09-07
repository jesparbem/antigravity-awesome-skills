---
name: btc-5m-live
description: Run and monitor BTC 5-minute Up/Down trading on Polymarket using momentum-near-close logic (entry window, BTC impulse confirmation, book skew), enforced risk caps, optional micro-hedge, and either a terminal or a hardened web console.
---

# BTC 5m Live

## Entry points
- Terminal dashboard: `python -m btc5m.cli tui --profile <name> --source <simulated|polymarket>`
- Web console: `uvicorn web.app:app` (see `README.md` for the required environment)
- Headless run: `python -m btc5m.cli run --profile <name>`
- PnL report: `python -m btc5m.cli report`
- Preflight: `python -m btc5m.cli doctor --source polymarket [--repo <trading repo>]`

## Strategy
Momentum into the close, as documented upstream — and, unlike v1, actually enforced:

- Enter inside the window `session_timing.min_entry_seconds_left` ..
  `max_entry_seconds_left` (default 90–150s, i.e. the ~120s target ±30s).
- Require a BTC spot move of at least `impulse_filter.btc_move_usd_min` inside
  the active slot.
- Take the side whose CLOB best ask clears `signal.threshold_price`; if both
  clear it, take the stronger.
- Refuse the trade when the book contradicts the spot move: follow momentum,
  never fade it.
- Size at the smallest of `stake_usd`, `max_notional_usd`,
  `risk_per_trade_pct_equity` of equity, and the depth resting at the best ask.
- Optional opposite micro-hedge when skew is extreme near the close.
- Exit on stop-loss from entry, or `exit_before_sec` before resolution.

## Operating rules
- Paper execution is the default. Live orders require `--execute`, an
  interactive confirmation, a profile other than `demo`, and a `BTC5M_REPO`
  checkout that passes preflight.
- `config/btc_5m_profiles.yaml` is the only source of strategy parameters. It is
  validated at load; a bad value aborts startup naming the YAML path.
- Daily loss and trade caps are enforced and survive a process restart.
- The kill switch blocks new entries immediately and is available from the web
  console even in read-only deployments.

## Configuration
- File: `config/btc_5m_profiles.yaml`
- Profiles: `conservative`, `aggressive`, `demo` (paper-only)
- `shared_rules` supplies defaults; profiles deep-merge overrides on top.

## Runtime artifacts
- Trade and event store: SQLite at `BTC5M_DB` (default `runtime/btc5m.db`)
- Logs: structured, with secrets redacted

## Notes
- Live order placement is delegated to `src/live/pm_live_trade_runner.py` in the
  private trading repo; this project never reads or logs credentials.
- `docs/CHANGES.md` records what changed against v1 and what was verified.
- Keep all GitHub-facing docs and metadata in English.
