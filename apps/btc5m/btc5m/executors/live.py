"""Live execution by delegating to the private Polymarket order runner.

This is the one place that can move real money, and it keeps v1's contract:
shell out to ``src/live/pm_live_trade_runner.py`` in the trading repo. What
changed is everything around that call.

* **Preflight.** The repo, interpreter and runner are checked once at
  construction. v1 discovered a wrong ``BTC5M_REPO`` as an opaque non-zero exit
  in the middle of a live session.
* **Timeouts.** Every subprocess is bounded. v1 used ``subprocess.run`` with no
  timeout, so a hung HTTP call inside the runner froze the session past market
  close with an open position.
* **Dry-run is the default.** ``--execute`` is appended only when this executor
  was explicitly constructed with ``live=True``.
* **No secret leakage.** The child inherits credentials through the environment,
  and only the parsed JSON result is ever logged or persisted — never the raw
  environment, and never the full stdout on the happy path.
"""

from __future__ import annotations

import json
import logging
import os
import subprocess
from pathlib import Path
from typing import Any, Optional

from ..errors import ExecutionError
from ..models import Fill, Market, Side
from .base import OpenRequest

log = logging.getLogger(__name__)

RUNNER_RELATIVE = Path("src/live/pm_live_trade_runner.py")


def extract_json_objects(text: str) -> list[dict[str, Any]]:
    """Pull top-level JSON objects out of mixed stdout.

    Same job as v1's hand-rolled brace counter, but string- and escape-aware:
    the old one miscounted braces inside quoted strings and silently dropped the
    order result whenever an error message happened to contain a ``{``.
    """
    out: list[dict[str, Any]] = []
    depth = 0
    start = -1
    in_string = False
    escaped = False
    for index, char in enumerate(text):
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}" and depth > 0:
            depth -= 1
            if depth == 0 and start >= 0:
                try:
                    parsed = json.loads(text[start : index + 1])
                except json.JSONDecodeError:
                    parsed = None
                if isinstance(parsed, dict):
                    out.append(parsed)
                start = -1
    return out


class LiveSubprocessExecutor:
    """Places orders via the external ``pm_live_trade_runner.py``."""

    name = "live"

    def __init__(
        self,
        *,
        repo: str | os.PathLike[str],
        python_bin: Optional[str] = None,
        live: bool = False,
        timeout_sec: float = 90.0,
        env_overrides: Optional[dict[str, str]] = None,
    ) -> None:
        self.repo = Path(repo).expanduser().resolve()
        self.live = bool(live)
        self.timeout_sec = float(timeout_sec)
        self.python_bin = python_bin or str(self.repo / ".venv" / "bin" / "python")
        self.env_overrides = dict(env_overrides or {})
        self._preflight()

    def _preflight(self) -> None:
        if not self.repo.is_dir():
            raise ExecutionError(
                f"trading repo not found: {self.repo}. Set BTC5M_REPO to the checkout that "
                "contains src/live/pm_live_trade_runner.py."
            )
        runner = self.repo / RUNNER_RELATIVE
        if not runner.is_file():
            raise ExecutionError(f"order runner not found: {runner}")
        if not Path(self.python_bin).is_file():
            raise ExecutionError(
                f"interpreter not found: {self.python_bin}. Create the trading repo virtualenv "
                "or set BTC5M_PYTHON."
            )

    # -- subprocess ---------------------------------------------------------- #
    def _run(self, args: list[str]) -> tuple[str, list[dict[str, Any]]]:
        cmd = [self.python_bin, str(RUNNER_RELATIVE), *args]
        if self.live:
            cmd.append("--execute")
        env = os.environ.copy()
        env.update(self.env_overrides)
        try:
            proc = subprocess.run(
                cmd, cwd=self.repo, capture_output=True, text=True,
                env=env, timeout=self.timeout_sec, check=False,
            )
        except subprocess.TimeoutExpired as exc:
            raise ExecutionError(
                f"order runner exceeded {self.timeout_sec:.0f}s and was killed: {' '.join(args)}"
            ) from exc
        combined = f"{proc.stdout or ''}\n{proc.stderr or ''}"
        objects = extract_json_objects(combined)
        if proc.returncode != 0 and not objects:
            raise ExecutionError(
                f"order runner exited {proc.returncode}: {combined.strip()[-500:]}"
            )
        return combined, objects

    @staticmethod
    def _matched(objects: list[dict[str, Any]]) -> tuple[Optional[dict[str, Any]], Optional[dict[str, Any]]]:
        for obj in reversed(objects):
            post = obj.get("order_post_result")
            if isinstance(post, dict):
                return obj, post
        return None, None

    # -- ExecutionEngine ----------------------------------------------------- #
    def open(self, request: OpenRequest) -> Optional[Fill]:
        args = [
            "--market-slug", request.market.slug,
            "--force-side", request.side.value,
            "--start-equity", "100",
            "--risk-frac", f"{request.stake_usd / 100.0:.6f}",
            "--max-notional-usd", f"{request.stake_usd:.4f}",
        ]
        _, objects = self._run(args)
        envelope, post = self._matched(objects)
        if not post or post.get("success") is not True:
            log.info("open not filled: %s", (post or {}).get("status"))
            return None
        if str(post.get("status", "")).lower() != "matched":
            return None

        shares = float(post.get("takingAmount") or 0)
        usdc = float(post.get("makingAmount") or 0)
        if shares <= 0:
            return None
        token = str((envelope or {}).get("token_id") or
                    (request.market.up_token if request.side is Side.UP else request.market.down_token))
        price = float((envelope or {}).get("entry_price") or (usdc / shares if shares else request.limit_price))
        hashes = post.get("transactionsHashes") or []
        return Fill(
            ts=request.at, side=request.side, token_id=token,
            price=round(price, 6), shares=round(shares, 6), usdc=round(usdc, 6),
            order_id=post.get("orderID"),
            tx_hash=hashes[0] if hashes else None,
            simulated=not self.live,
        )

    def close(self, *, market: Market, side: Side, token_id: str, shares: float,
              mark: Optional[float], at: float) -> Optional[Fill]:
        """Close with a marketable order, falling back to a limit at the bid.

        v1 escalated FAK -> GTC -> cancel -> aggressive GTC inline in the session
        loop across ~120 lines. The escalation is kept, but as two bounded steps
        driven by the caller's retry budget instead of nested branches.
        """
        args = [
            "--market-slug", market.slug,
            "--close-token-id", token_id,
            "--close-shares", f"{shares:.8f}",
        ]
        self.env_overrides["PM_CLOSE_ORDER_TYPE"] = "FAK"
        _, objects = self._run(args)
        _, post = self._matched(objects)

        if not (post and post.get("success") is True and str(post.get("status", "")).lower() == "matched"):
            if mark is None:
                return None
            # Fall back to a limit one tick inside the bid so it rests marketably.
            limit = max(0.01, min(0.99, mark - 0.01))
            self.env_overrides["PM_CLOSE_ORDER_TYPE"] = "GTC"
            _, objects = self._run([*args, "--close-limit-price", f"{limit:.6f}"])
            _, post = self._matched(objects)
            if not (post and post.get("success") is True
                    and str(post.get("status", "")).lower() == "matched"):
                return None

        usdc = float(post.get("takingAmount") or 0)
        filled = float(post.get("makingAmount") or 0) or shares
        hashes = post.get("transactionsHashes") or []
        return Fill(
            ts=at, side=side, token_id=token_id,
            price=round(usdc / filled, 6) if filled else 0.0,
            shares=round(filled, 6), usdc=round(usdc, 6),
            order_id=post.get("orderID"),
            tx_hash=hashes[0] if hashes else None,
            simulated=not self.live,
        )
