# What changed against v1

Reference: [Novals83/5min-btc-polymarket@1c9aa81](https://github.com/Novals83/5min-btc-polymarket/commit/1c9aa81ec64cdf62895b155ed64dc8e88c013102).

## Correctness

**The profile file was ignored.** `config/btc_5m_profiles.yaml` defined hedging,
spread guards, liquidity guards, daily loss caps and trade ceilings.
`scripts/test_btc_5m_session_exit_sl.py` carried its own hard-coded `PROFILES`
dict with seven keys and read the YAML never. Editing the config changed
nothing. The YAML is now the only source of truth, deep-merged from
`shared_rules` and validated field by field.

**The entry window had no upper bound.** The strategy targets ~120s left with a
30s tolerance. v1 checked only `sec_left >= min_entry_seconds_left` (60), so a
session started at 4 minutes left entered immediately on any qualifying price.
`max_entry_seconds_left` now closes the window.

**The impulse filter did not exist.** "Confirm BTC has moved $70–$100" appears in
the README, `SKILL.md` and the YAML. There was no spot price feed in the
codebase, so the rule was unimplementable. Added `sources/price.py` with
Binance, Coinbase and offline feeds, plus an `ImpulseTracker` that measures the
move from the first sample of the active slot.

**Momentum could be faded.** The side was chosen purely by whichever ask was
higher, with no reference to spot. If BTC had moved down $150 while the book was
bid for UP, v1 bought UP. Entry is now refused on a direction mismatch.

**Liquidity could not be checked.** `skip_if_top_ask_notional_usd_lt` needs
resting size. v1 used `py_clob_client` and kept only prices from the book. The
CLOB REST book is now read directly, so depth is available and the guard works —
and the stake is capped at what the top of book can actually fill.

**Positions were marked off the wrong price.** Stop-losses compared against the
Gamma *outcome price*, an indicative mid. Marks now come from the best bid: the
price the position could actually be sold at.

**The JSON parser miscounted braces.** `parse_json_objects` counted `{` and `}`
without tracking string literals or escapes, so any error message containing a
brace desynchronised it and the order result was silently dropped. Replaced with
a string- and escape-aware scanner (tested against both cases).

**Risk caps were decorative.** `daily_max_loss_pct`, `max_trades_per_day` and
`skip_if_dns_or_api_errors_consecutive` were configured and never enforced.
`risk.py` enforces all three — and seeds itself from the day's recorded trades at
startup, so a restart no longer resets the daily loss cap to zero.

**Subprocesses could hang forever.** `subprocess.run` was called with no
timeout. One stuck HTTP call inside the order runner froze the session past
market close holding an open position. Every call is now bounded and the timeout
is reported as a typed error.

**A failed close abandoned the position.** After exhausting its retry budget v1
fell through, printed a report and exited with the position still open. The
engine now keeps the position and retries on the next tick.

**Errors were indistinguishable.** A bare `except Exception` around the whole
loop meant a bad profile, a DNS failure and a logic bug all looked the same.
Replaced with a typed hierarchy (`ConfigError`, `TransientSourceError`,
`ExecutionError`), and only transient errors feed the circuit breaker.

**Dead configuration is now rejected.** The shipped profiles had a hedge trigger
at `seconds_left <= 45` and, once the entry window was corrected, entries
stopping at 90s — so the hedge could never fire. The loader now refuses that
combination instead of shipping a parameter that does nothing.

## Runnability

Every v1 execution path required a private sibling repository
(`pm-hl-conservative-plus-repo`), its virtualenv, and funded Polymarket
credentials. Nobody else could run, review or test any of it.

* `executors/paper.py` — simulated fills with slippage and fees, exercising the
  full pipeline. Paper PnL is pessimistic, not flattering.
* `sources/simulated.py` — a self-consistent offline market whose book prices are
  derived from the same synthetic spot curve the impulse filter reads,
  calibrated so the in-slot move has a ~$65 median at the strategy's target
  moment.
* Live execution keeps the original delegation, behind preflight checks and two
  explicit opt-ins.

## Structure

`test_btc_5m_session_exit_sl.py` was 673 lines: HTTP, subprocess management,
strategy, sizing, exits and a 120-line close-escalation ladder in one `main()`
with two unbounded `while` loops. Nothing was reachable from a test.

Now: pure `strategy` and `risk` over frozen dataclasses; an `engine.Session`
whose `tick()` advances one step and publishes an immutable `Snapshot`; sources
and executors behind protocols. Both front-ends render the same snapshot, so
they cannot drift.

## Operations

* SQLite store written as events happen, replacing "dump one JSON blob at exit,
  then re-parse the tail of a log file to build reports".
* Structured logging with a redaction filter that masks private keys, API
  secrets and bearer tokens — including the live values read from the
  environment.
* `btc5m doctor` preflight command.
* Dockerfile and compose stack: unprivileged UID, read-only root filesystem, all
  capabilities dropped, `no-new-privileges`, loopback-bound port, healthcheck.
* 121 tests that need no network, credentials or trading repo.

## Web front-end (new)

A hardened FastAPI console: Argon2id credentials with no user enumeration,
server-side sessions with idle and absolute timeouts, CSRF on every state
change, per-IP login lockout and API rate limiting, a strict CSP with no inline
script or style, read-only by default, and a kill switch that stays available
even when controls are disabled.

## Terminal front-end (new)

A `rich` dashboard over the same snapshot: slot countdown, spot and in-slot
move, both sides of the book with spread and depth, the decision *and its coded
reason*, the open position with its stop, and the live risk ledger.
`tools/capture_tui.py` renders it to SVG without a TTY.

## Not changed

* The strategy itself: momentum into the close, threshold on the CLOB best ask,
  stop-loss from entry, time exit before resolution, optional extreme-skew hedge.
* Live order placement still delegates to `pm_live_trade_runner.py`.
* MIT license.

## Verified here

* 121 tests pass.
* The web console was driven end to end in headless Chromium: login, start a
  session, watch it trade, kill switch — no console errors, so the strict CSP
  does not break the page.
* `docker compose config` validates.

## Not verified here

* **Live Polymarket data.** `gamma-api.polymarket.com` and
  `clob.polymarket.com` are blocked by network policy in the environment this
  was built in, so `--source polymarket` is exercised only against unit tests
  and never against the real API. Run `btc5m doctor --source polymarket` from
  your own machine first.
* **The Docker image build.** Docker Hub blob fetches are blocked by the same
  policy. The Dockerfile parses and the compose file validates, but the image
  was never built here — build it once locally before deploying.
* **Live order placement**, which needs the private trading repo and funded
  credentials.
