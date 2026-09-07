"""SQLite persistence for trades and session events.

v1 "persisted" by writing the whole run as one giant JSON blob at process exit
and then re-parsing the tail of a log file to build reports — so a crashed or
still-running session had no recoverable history, and the PnL report was a
best-effort scrape. Rows are written as they happen instead.
"""

from __future__ import annotations

import json
import sqlite3
import threading
from pathlib import Path
from typing import Any, Iterable, Optional

from .models import TradeResult, iso

SCHEMA = """
CREATE TABLE IF NOT EXISTS trades (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    market_slug   TEXT    NOT NULL,
    profile       TEXT    NOT NULL,
    side          TEXT    NOT NULL,
    opened_at     REAL    NOT NULL,
    closed_at     REAL    NOT NULL,
    entry_price   REAL    NOT NULL,
    exit_price    REAL,
    shares        REAL    NOT NULL,
    cost_usdc     REAL    NOT NULL,
    proceeds_usdc REAL    NOT NULL,
    hedge_usdc    REAL    NOT NULL DEFAULT 0,
    pnl_usdc      REAL    NOT NULL,
    close_reason  TEXT    NOT NULL,
    simulated     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trades_closed_at ON trades(closed_at);

CREATE TABLE IF NOT EXISTS events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    ts      REAL NOT NULL,
    kind    TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
"""


class Store:
    """Small synchronous SQLite wrapper, safe to share across threads."""

    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._conn = sqlite3.connect(str(self.path), check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        with self._lock:
            self._conn.executescript(SCHEMA)
            self._conn.commit()

    def close(self) -> None:
        with self._lock:
            self._conn.close()

    # -- writes -------------------------------------------------------------- #
    def record_trade(self, result: TradeResult) -> int:
        with self._lock:
            cursor = self._conn.execute(
                """INSERT INTO trades (market_slug, profile, side, opened_at, closed_at,
                                       entry_price, exit_price, shares, cost_usdc,
                                       proceeds_usdc, hedge_usdc, pnl_usdc, close_reason, simulated)
                   VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
                (
                    result.market_slug, result.profile, result.side.value,
                    result.opened_at, result.closed_at, result.entry_price, result.exit_price,
                    result.shares, result.cost_usdc, result.proceeds_usdc, result.hedge_usdc,
                    result.pnl_usdc, result.close_reason, int(result.simulated),
                ),
            )
            self._conn.commit()
            return int(cursor.lastrowid or 0)

    def record_event(self, ts: float, kind: str, payload: dict[str, Any]) -> None:
        with self._lock:
            self._conn.execute(
                "INSERT INTO events (ts, kind, payload) VALUES (?,?,?)",
                (ts, kind, json.dumps(payload, ensure_ascii=False, default=str)),
            )
            self._conn.commit()

    def prune_events(self, keep: int = 5000) -> None:
        """Bound the event log so a long-running session cannot fill the disk."""
        with self._lock:
            self._conn.execute(
                "DELETE FROM events WHERE id NOT IN (SELECT id FROM events ORDER BY id DESC LIMIT ?)",
                (keep,),
            )
            self._conn.commit()

    # -- reads --------------------------------------------------------------- #
    def recent_trades(self, limit: int = 50) -> list[dict[str, Any]]:
        with self._lock:
            rows: Iterable[sqlite3.Row] = self._conn.execute(
                "SELECT * FROM trades ORDER BY closed_at DESC LIMIT ?", (limit,)
            ).fetchall()
        out = []
        for row in rows:
            item = dict(row)
            item["simulated"] = bool(item["simulated"])
            item["opened_iso"] = iso(item["opened_at"])
            item["closed_iso"] = iso(item["closed_at"])
            out.append(item)
        return out

    def recent_events(self, limit: int = 100, kind: Optional[str] = None) -> list[dict[str, Any]]:
        query = "SELECT * FROM events"
        params: list[Any] = []
        if kind:
            query += " WHERE kind = ?"
            params.append(kind)
        query += " ORDER BY id DESC LIMIT ?"
        params.append(limit)
        with self._lock:
            rows = self._conn.execute(query, params).fetchall()
        return [
            {"ts": r["ts"], "ts_iso": iso(r["ts"]), "kind": r["kind"], "payload": json.loads(r["payload"])}
            for r in rows
        ]

    def stats(self, since_ts: Optional[float] = None) -> dict[str, Any]:
        query = ("SELECT COUNT(*) n, COALESCE(SUM(pnl_usdc),0) pnl, "
                 "COALESCE(SUM(CASE WHEN pnl_usdc > 0 THEN 1 ELSE 0 END),0) wins, "
                 "COALESCE(SUM(cost_usdc),0) volume FROM trades")
        params: list[Any] = []
        if since_ts is not None:
            query += " WHERE closed_at >= ?"
            params.append(since_ts)
        with self._lock:
            row = self._conn.execute(query, params).fetchone()
        trades = int(row["n"])
        wins = int(row["wins"])
        return {
            "trades": trades,
            "wins": wins,
            "losses": trades - wins,
            "win_rate_pct": round(wins / trades * 100, 2) if trades else None,
            "pnl_usdc": round(float(row["pnl"]), 6),
            "volume_usdc": round(float(row["volume"]), 6),
        }
