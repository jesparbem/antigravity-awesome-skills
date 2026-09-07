/* btc5m operator console.
 *
 * Polls /api/state and re-renders. Every value coming back from the API is
 * written with textContent, never innerHTML: the payload embeds market slugs
 * and error strings from upstream services, and none of it is trusted markup.
 */
"use strict";

const POLL_MS = 2000;
const csrf = document.querySelector('meta[name="csrf-token"]').content;
const readOnly = document.querySelector('meta[name="read-only"]').content === "true";

const $ = (id) => document.getElementById(id);

const REASONS = {
  signal_confirmed: "Signal confirmed — entering",
  holding_position: "Holding — watching stop and clock",
  closed: "Position closed",
  too_early_to_enter: "Waiting for the entry window",
  too_late_to_enter: "Past the entry window",
  price_below_threshold: "No side above the price threshold",
  impulse_too_small: "BTC move below the impulse minimum",
  impulse_unavailable: "Spot feed unavailable",
  impulse_direction_mismatch: "Book disagrees with the move — not fading it",
  spread_too_wide: "Spread above the guard",
  insufficient_liquidity: "Top of book too thin",
  liquidity_unknown: "Book depth unknown",
  quote_stale: "Quote too old",
  market_not_tradable: "Market closed or inactive",
  no_active_market: "No active 5m slot",
  no_quote: "No order book",
  stake_is_zero: "Sizing resolved to zero",
  kill_switch: "KILL SWITCH ENGAGED",
  daily_loss_limit: "Daily loss cap reached",
  max_trades_per_day: "Daily trade cap reached",
  source_circuit_breaker: "Market data circuit breaker open",
};

const fmt = {
  price: (v) => (v === null || v === undefined ? "—" : Number(v).toFixed(3)),
  usd: (v) =>
    v === null || v === undefined
      ? "—"
      : `$${Number(v).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`,
  signed: (v) => (v === null || v === undefined ? "—" : `${Number(v) >= 0 ? "+" : ""}${Number(v).toFixed(4)}`),
  signedUsd: (v) =>
    v === null || v === undefined
      ? "—"
      : `${Number(v) >= 0 ? "+" : "-"}$${Math.abs(Number(v)).toLocaleString(undefined, {
          minimumFractionDigits: 2,
          maximumFractionDigits: 2,
        })}`,
  time: (iso) => (iso ? iso.slice(11, 19) + "Z" : "—"),
};

function setSigned(el, value, suffix = "") {
  el.textContent = value === null || value === undefined ? "—" : fmt.signed(value) + suffix;
  el.classList.toggle("pos", Number(value) > 0);
  el.classList.toggle("neg", Number(value) < 0);
}

function setBar(el, fraction, warnAt = 0.5, badAt = 0.8) {
  const f = Math.max(0, Math.min(1, fraction || 0));
  el.style.width = `${f * 100}%`;
  el.classList.toggle("warn", f >= warnAt && f < badAt);
  el.classList.toggle("bad", f >= badAt);
}

function renderMarket(state) {
  const market = state.market;
  if (!market) {
    $("m-slug").textContent = "—";
    $("m-end").textContent = "—";
    $("m-left").textContent = "no active slot";
    setBar($("m-bar"), 0);
    return;
  }
  const left = state.seconds_left ?? 0;
  $("m-slug").textContent = market.slug;
  $("m-end").textContent = fmt.time(market.end_iso);
  $("m-left").textContent = `${left.toFixed(1)}s`;
  setBar($("m-bar"), 1 - left / 300, 0.7, 0.9);

  $("m-spot").textContent = state.spot ? fmt.usd(state.spot) : "—";
  const impulse = state.impulse;
  if (impulse) {
    const move = impulse.move_usd;
    const el = $("m-move");
    el.textContent = `${move > 0 ? "▲" : move < 0 ? "▼" : "="} ${fmt.signedUsd(move)}`;
    el.classList.toggle("pos", move > 0);
    el.classList.toggle("neg", move < 0);
  } else {
    $("m-move").textContent = "—";
  }
}

function renderBook(state) {
  const body = $("book-body");
  body.replaceChildren();
  const quote = state.quote;
  if (!quote) {
    const row = body.insertRow();
    const cell = row.insertCell();
    cell.colSpan = 5;
    cell.className = "muted";
    cell.textContent = "no order book";
    return;
  }
  for (const side of ["UP", "DOWN"]) {
    const key = side.toLowerCase();
    const bid = quote[`${key}_bid`];
    const ask = quote[`${key}_ask`];
    const depth = quote[`${key}_ask_notional`];
    const spread = bid !== null && ask !== null ? ask - bid : null;

    const row = body.insertRow();
    const cells = [
      side,
      fmt.price(bid),
      fmt.price(ask),
      spread === null ? "—" : spread.toFixed(3),
      depth === null || depth === undefined ? "—" : fmt.usd(depth),
    ];
    cells.forEach((value, index) => {
      const cell = row.insertCell();
      cell.textContent = value;
      if (index > 0) cell.className = "num";
      if (index === 2 && ask !== null && ask >= 0.7) cell.classList.add("pos", "strong");
    });
  }
}

function renderDecision(state) {
  const decision = state.decision;
  const blocked = state.block_reason;
  const dot = $("d-dot");
  const action = blocked ? "BLOCKED" : decision ? decision.action.toUpperCase() : "—";
  const reasonKey = blocked || (decision && decision.reason) || "";

  $("d-action").textContent = action;
  $("d-reason").textContent = REASONS[reasonKey] || reasonKey || "waiting for the first tick…";
  dot.className = "dot " + (blocked ? "block" : decision ? decision.action : "");

  const detail = $("d-detail");
  detail.replaceChildren();
  if (!decision) return;
  const rows = {};
  if (decision.side) rows["Side"] = decision.side;
  if (decision.price !== null && decision.price !== undefined) rows["Price"] = fmt.price(decision.price);
  if (decision.action === "enter") {
    rows["Stake"] = fmt.usd(decision.stake_usd);
    if (decision.hedge_usd) rows["Hedge"] = fmt.usd(decision.hedge_usd);
  }
  for (const [key, value] of Object.entries(decision.detail || {}).slice(0, 4)) {
    rows[key.replace(/_/g, " ")] = String(value);
  }
  for (const [key, value] of Object.entries(rows)) {
    const dt = document.createElement("dt");
    dt.textContent = key;
    const dd = document.createElement("dd");
    dd.textContent = value;
    detail.append(dt, dd);
  }
}

function renderPosition(state) {
  const position = state.position;
  const empty = $("p-empty");
  const detail = $("p-detail");
  if (!position) {
    empty.hidden = false;
    detail.hidden = true;
    return;
  }
  empty.hidden = true;
  detail.hidden = false;
  $("p-side").textContent = position.side;
  setSigned($("p-unreal"), state.unrealised_usdc, " USDC");
  $("p-prices").textContent = `${fmt.price(position.entry_price)} → ${fmt.price(state.mark)}`;
  $("p-stop").textContent = fmt.price(position.stop_loss_price);
  $("p-size").textContent = `${position.shares.toFixed(4)} sh / ${fmt.usd(position.cost_usdc)}`;
}

function renderRisk(state) {
  const risk = state.risk || {};
  $("r-equity").textContent = fmt.usd(risk.equity_usd);
  setSigned($("r-pnl"), risk.realised_pnl_usdc, " USDC");

  const trades = risk.trades_today || 0;
  const cap = risk.max_trades_per_day || 1;
  $("r-trades").textContent = `${trades} / ${cap}`;
  setBar($("r-trades-bar"), trades / cap, 0.6, 0.85);

  const used = risk.daily_loss_used_pct || 0;
  $("r-loss").textContent = `${used.toFixed(1)}%`;
  setBar($("r-loss-bar"), used / 100, 0.5, 0.8);

  const kill = $("r-kill");
  kill.textContent = risk.kill_switch ? `ENGAGED — ${risk.kill_switch_reason || ""}` : "clear";
  kill.classList.toggle("neg", !!risk.kill_switch);
}

function renderHeader(state) {
  const mode = $("mode-pill");
  mode.textContent = state.mode;
  mode.className = "pill " + (state.mode === "live" ? "live" : "ok");

  const run = $("run-pill");
  run.textContent = state.running ? "running" : "stopped";
  run.className = "pill " + (state.running ? "ok" : "");

  $("subtitle").textContent = `profile ${state.profile} · tick ${state.ticks} · ${fmt.time(state.ts_iso)}`;
  $("last-error").textContent = state.last_error ? `last error: ${state.last_error}` : "";
}

function renderTrades(report) {
  const body = $("trades-body");
  body.replaceChildren();
  const trades = report.trades || [];
  if (!trades.length) {
    const row = body.insertRow();
    const cell = row.insertCell();
    cell.colSpan = 7;
    cell.className = "muted";
    cell.textContent = "No trades yet.";
  } else {
    for (const trade of trades.slice(0, 12)) {
      const row = body.insertRow();
      const cells = [
        fmt.time(trade.closed_iso),
        trade.market_slug,
        trade.side,
        fmt.price(trade.entry_price),
        fmt.price(trade.exit_price),
        fmt.signed(trade.pnl_usdc),
        trade.close_reason,
      ];
      cells.forEach((value, index) => {
        const cell = row.insertCell();
        cell.textContent = value;
        if (index === 5) cell.classList.add("num", trade.pnl_usdc >= 0 ? "pos" : "neg");
        else if (index === 3 || index === 4) cell.classList.add("num");
      });
    }
  }
  const overall = report.overall || {};
  $("trades-summary").textContent = overall.trades
    ? `${overall.trades} trades · ${overall.win_rate_pct}% win rate · ${fmt.signed(overall.pnl_usdc)} USDC realised`
    : "";
}

async function control(action) {
  const status = $("control-status");
  status.textContent = `${action}…`;
  try {
    const response = await fetch("/api/control", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      body: JSON.stringify({ action, csrf_token: csrf, profile: $("profile-select").value }),
    });
    const payload = await response.json();
    status.textContent = response.ok ? `${action}: ${payload.result}` : `${action} failed: ${payload.error}`;
  } catch (error) {
    status.textContent = `${action} failed: ${error.message}`;
  }
  refresh();
}

async function refresh() {
  try {
    const [stateResponse, reportResponse] = await Promise.all([
      fetch("/api/state", { headers: { Accept: "application/json" } }),
      fetch("/api/report?limit=12", { headers: { Accept: "application/json" } }),
    ]);
    if (stateResponse.status === 401) {
      window.location.href = "/login";
      return;
    }
    if (!stateResponse.ok) return;
    const state = await stateResponse.json();
    renderHeader(state);
    renderMarket(state);
    renderBook(state);
    renderDecision(state);
    renderPosition(state);
    renderRisk(state);
    if (reportResponse.ok) renderTrades(await reportResponse.json());
  } catch (error) {
    $("last-error").textContent = `poll failed: ${error.message}`;
  }
}

document.querySelectorAll("button[data-action]").forEach((button) => {
  button.addEventListener("click", () => control(button.dataset.action));
});

refresh();
setInterval(refresh, POLL_MS);
