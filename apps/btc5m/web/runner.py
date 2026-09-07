"""Background session supervision for the web front-end.

The engine is synchronous by design, so the web app runs it on a worker thread
and only ever reads its published snapshot. Nothing in a request handler blocks
on market data.
"""

from __future__ import annotations

import logging
import threading
import time
from typing import Any, Optional

from btc5m.config import ProfileSet
from btc5m.engine import Session, Snapshot
from btc5m.errors import Btc5mError
from btc5m.factory import build_session
from btc5m.store import Store

log = logging.getLogger(__name__)


class SessionRunner:
    """Owns at most one live :class:`Session` and the thread driving it."""

    def __init__(self, *, profiles: ProfileSet, profile_name: str, source: str,
                 equity_usd: float, db_path: str, live: bool = False) -> None:
        self.profiles = profiles
        self.source = source
        self.equity_usd = equity_usd
        self.db_path = db_path
        self.live = live
        self._lock = threading.Lock()
        self._thread: Optional[threading.Thread] = None
        self._session: Optional[Session] = None
        self._store: Optional[Store] = None
        self._error: Optional[str] = None
        self._build(profile_name)

    # -- lifecycle ----------------------------------------------------------- #
    def _build(self, profile_name: str) -> None:
        profile = self.profiles.get(profile_name)
        session, store = build_session(
            profile=profile, source_name=self.source, live=self.live,
            equity_usd=self.equity_usd, db_path=self.db_path,
        )
        self._session = session
        self._store = store

    @property
    def session(self) -> Session:
        assert self._session is not None
        return self._session

    @property
    def store(self) -> Store:
        assert self._store is not None
        return self._store

    @property
    def running(self) -> bool:
        return bool(self._thread and self._thread.is_alive())

    @property
    def error(self) -> Optional[str]:
        return self._error

    def start(self, profile_name: Optional[str] = None) -> str:
        with self._lock:
            if self.running:
                return "already_running"
            if profile_name and profile_name != self.session.profile.name:
                self.store.close()
                self._build(profile_name)
            self._error = None
            session = self.session
            deadline = time.time() + session.profile.runner.entry_timeout_min * 60

            def drive() -> None:
                try:
                    session.run(deadline_ts=deadline)
                except Btc5mError as exc:
                    self._error = str(exc)
                    log.error("session stopped: %s", exc)
                except Exception as exc:  # never let a worker die silently
                    self._error = f"unexpected error: {exc}"
                    log.exception("session crashed")

            self._thread = threading.Thread(target=drive, name="btc5m-session", daemon=True)
            self._thread.start()
            return "started"

    def stop(self, timeout: float = 10.0) -> str:
        with self._lock:
            if not self.running:
                return "already_stopped"
            self.session.request_stop()
            thread = self._thread
        if thread:
            thread.join(timeout=timeout)
        return "stopped"

    def switch_profile(self, profile_name: str) -> str:
        if self.running:
            return "refused_running"
        with self._lock:
            self.store.close()
            self._build(profile_name)
        return "switched"

    def engage_kill_switch(self, reason: str = "operator") -> str:
        self.session.risk.engage_kill_switch(reason)
        return "engaged"

    def release_kill_switch(self) -> str:
        self.session.risk.release_kill_switch()
        return "released"

    # -- observation --------------------------------------------------------- #
    def snapshot(self) -> Snapshot:
        return self.session.snapshot

    def state(self) -> dict[str, Any]:
        snap = self.snapshot()
        data = snap.as_dict()
        # Risk is read live rather than from the snapshot: an operator action
        # (kill switch) must show up immediately even while the session is
        # stopped and publishing no new snapshots.
        data["risk"] = self.session.risk.as_dict()
        data["running"] = self.running
        data["error"] = self._error
        data["available_profiles"] = self.profiles.names
        return data

    def close(self) -> None:
        self.stop()
        if self._store is not None:
            self._store.close()
