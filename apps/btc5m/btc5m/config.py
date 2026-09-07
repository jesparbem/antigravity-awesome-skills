"""Profile loading and validation.

The v1 skill shipped ``config/btc_5m_profiles.yaml`` *and* a hard-coded
``PROFILES`` dict inside the runner. Only the dict was ever read, so editing the
YAML changed nothing: hedging, spread guards, liquidity guards and daily loss
caps were documented but unreachable. This module makes the YAML the single
source of truth, deep-merges ``shared_rules`` under each profile, validates
every field, and fails loudly with the offending path instead of silently
falling back to defaults.
"""

from __future__ import annotations

import copy
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Mapping, Optional

import yaml

from .errors import ConfigError

DEFAULT_CONFIG_PATH = Path(__file__).resolve().parents[1] / "config" / "btc_5m_profiles.yaml"


# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #
def _deep_merge(base: Mapping[str, Any], override: Mapping[str, Any]) -> dict[str, Any]:
    """Recursively merge ``override`` onto ``base`` without mutating either."""
    out = copy.deepcopy(dict(base))
    for key, value in (override or {}).items():
        if isinstance(value, Mapping) and isinstance(out.get(key), Mapping):
            out[key] = _deep_merge(out[key], value)
        else:
            out[key] = copy.deepcopy(value)
    return out


def _num(section: Mapping[str, Any], key: str, default: float, *, path: str,
         minimum: Optional[float] = None, maximum: Optional[float] = None) -> float:
    raw = section.get(key, default)
    if isinstance(raw, bool) or not isinstance(raw, (int, float)):
        raise ConfigError(f"{path}.{key}: expected a number, got {raw!r}")
    value = float(raw)
    if minimum is not None and value < minimum:
        raise ConfigError(f"{path}.{key}: {value} is below the minimum {minimum}")
    if maximum is not None and value > maximum:
        raise ConfigError(f"{path}.{key}: {value} is above the maximum {maximum}")
    return value


def _int(section: Mapping[str, Any], key: str, default: int, *, path: str,
         minimum: Optional[int] = None, maximum: Optional[int] = None) -> int:
    return int(_num(section, key, default, path=path, minimum=minimum, maximum=maximum))


def _bool(section: Mapping[str, Any], key: str, default: bool, *, path: str) -> bool:
    raw = section.get(key, default)
    if not isinstance(raw, bool):
        raise ConfigError(f"{path}.{key}: expected true/false, got {raw!r}")
    return raw


def _str(section: Mapping[str, Any], key: str, default: str, *, path: str,
         choices: Optional[tuple[str, ...]] = None) -> str:
    raw = section.get(key, default)
    if not isinstance(raw, str):
        raise ConfigError(f"{path}.{key}: expected a string, got {raw!r}")
    if choices and raw not in choices:
        raise ConfigError(f"{path}.{key}: {raw!r} is not one of {choices}")
    return raw


# --------------------------------------------------------------------------- #
# sections
# --------------------------------------------------------------------------- #
@dataclass(frozen=True)
class SessionTiming:
    """When an entry is allowed inside a 5-minute slot.

    ``max_entry_seconds_left`` is new. The strategy targets ~120s left with a
    30s tolerance, but v1 only enforced a *lower* bound, so a session starting
    at 280s left would enter immediately on any qualifying price — far outside
    the documented window.
    """

    min_entry_seconds_left: int = 60
    max_entry_seconds_left: int = 150
    exit_before_sec: int = 20

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "SessionTiming":
        obj = cls(
            min_entry_seconds_left=_int(raw, "min_entry_seconds_left", 60, path=path, minimum=0, maximum=300),
            max_entry_seconds_left=_int(raw, "max_entry_seconds_left", 150, path=path, minimum=1, maximum=300),
            exit_before_sec=_int(raw, "exit_before_sec", 20, path=path, minimum=0, maximum=300),
        )
        if obj.max_entry_seconds_left < obj.min_entry_seconds_left:
            raise ConfigError(
                f"{path}: max_entry_seconds_left ({obj.max_entry_seconds_left}) must be >= "
                f"min_entry_seconds_left ({obj.min_entry_seconds_left})"
            )
        if obj.exit_before_sec >= obj.min_entry_seconds_left:
            raise ConfigError(
                f"{path}: exit_before_sec ({obj.exit_before_sec}) must be < "
                f"min_entry_seconds_left ({obj.min_entry_seconds_left}), otherwise every "
                "entry is closed on the very next tick"
            )
        return obj


@dataclass(frozen=True)
class ExecutionSafety:
    """Microstructure guards evaluated on every tick."""

    skip_if_quote_stale_sec_gt: float = 8.0
    skip_if_dns_or_api_errors_consecutive: int = 3
    skip_if_spread_gt: float = 0.03
    skip_if_top_ask_notional_usd_lt: float = 30.0
    require_liquidity_data: bool = True

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "ExecutionSafety":
        return cls(
            skip_if_quote_stale_sec_gt=_num(raw, "skip_if_quote_stale_sec_gt", 8.0, path=path, minimum=0.5),
            skip_if_dns_or_api_errors_consecutive=_int(
                raw, "skip_if_dns_or_api_errors_consecutive", 3, path=path, minimum=1
            ),
            skip_if_spread_gt=_num(raw, "skip_if_spread_gt", 0.03, path=path, minimum=0.0, maximum=1.0),
            skip_if_top_ask_notional_usd_lt=_num(
                raw, "skip_if_top_ask_notional_usd_lt", 30.0, path=path, minimum=0.0
            ),
            require_liquidity_data=_bool(raw, "require_liquidity_data", True, path=path),
        )


@dataclass(frozen=True)
class ImpulseFilter:
    """The ~$70-$100 BTC move confirmation from the published strategy.

    v1 documented it in three places and implemented it nowhere, because the
    runner had no spot price feed at all. ``require_direction_match`` enforces
    the "follow momentum, never fade it" rule.
    """

    enabled: bool = True
    source: str = "binance"
    symbol: str = "BTCUSDT"
    btc_move_usd_min: float = 70.0
    btc_move_usd_max_reference: float = 100.0
    require_direction_match: bool = True
    min_samples: int = 3
    fail_action: str = "skip"

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "ImpulseFilter":
        return cls(
            enabled=_bool(raw, "enabled", True, path=path),
            source=_str(raw, "source", "binance", path=path, choices=("binance", "coinbase", "simulated")),
            symbol=_str(raw, "symbol", "BTCUSDT", path=path),
            btc_move_usd_min=_num(raw, "btc_move_usd_min", 70.0, path=path, minimum=0.0),
            btc_move_usd_max_reference=_num(raw, "btc_move_usd_max_reference", 100.0, path=path, minimum=0.0),
            require_direction_match=_bool(raw, "require_direction_match", True, path=path),
            min_samples=_int(raw, "min_samples", 3, path=path, minimum=1),
            fail_action=_str(raw, "fail_action", "skip", path=path, choices=("skip", "ignore")),
        )


@dataclass(frozen=True)
class Signal:
    trigger_source: str = "clob_best_ask"
    threshold_price: float = 0.70

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "Signal":
        return cls(
            trigger_source=_str(
                raw, "trigger_source", "clob_best_ask", path=path,
                choices=("clob_best_ask", "gamma_outcome_price"),
            ),
            threshold_price=_num(raw, "threshold_price", 0.70, path=path, minimum=0.01, maximum=0.99),
        )


@dataclass(frozen=True)
class Sizing:
    stake_usd: float = 5.0
    risk_per_trade_pct_equity: float = 8.0
    max_notional_usd: float = 8.0
    daily_max_loss_pct: float = 10.0
    max_trades_per_day: int = 12

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "Sizing":
        return cls(
            stake_usd=_num(raw, "stake_usd", 5.0, path=path, minimum=0.5),
            risk_per_trade_pct_equity=_num(raw, "risk_per_trade_pct_equity", 8.0, path=path, minimum=0.01, maximum=100.0),
            max_notional_usd=_num(raw, "max_notional_usd", 8.0, path=path, minimum=0.5),
            daily_max_loss_pct=_num(raw, "daily_max_loss_pct", 10.0, path=path, minimum=0.1, maximum=100.0),
            max_trades_per_day=_int(raw, "max_trades_per_day", 12, path=path, minimum=1),
        )

    def stake_for(self, equity_usd: float) -> float:
        """Smallest of the three configured caps. All three were ignored in v1."""
        by_pct = equity_usd * (self.risk_per_trade_pct_equity / 100.0)
        return round(max(0.0, min(self.stake_usd, self.max_notional_usd, by_pct)), 4)


@dataclass(frozen=True)
class Hedge:
    """Extreme-skew micro-hedge, evaluated at entry.

    ``trigger_seconds_left_lte`` must sit inside the entry window or the hedge
    is unreachable; :func:`load_profiles` rejects that combination, so the
    default here is chosen to be coherent with ``SessionTiming``'s defaults.
    """

    enabled: bool = True
    trigger_side_price_gte: float = 0.95
    trigger_seconds_left_lte: int = 120
    hedge_share_of_main_pct: float = 3.0
    hedge_notional_usd_min: float = 1.0
    hedge_notional_usd_max: float = 2.0

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "Hedge":
        obj = cls(
            enabled=_bool(raw, "enabled", True, path=path),
            trigger_side_price_gte=_num(raw, "trigger_side_price_gte", 0.95, path=path, minimum=0.5, maximum=0.999),
            trigger_seconds_left_lte=_int(raw, "trigger_seconds_left_lte", 120, path=path, minimum=0, maximum=300),
            hedge_share_of_main_pct=_num(raw, "hedge_share_of_main_pct", 3.0, path=path, minimum=0.0, maximum=100.0),
            hedge_notional_usd_min=_num(raw, "hedge_notional_usd_min", 1.0, path=path, minimum=0.0),
            hedge_notional_usd_max=_num(raw, "hedge_notional_usd_max", 2.0, path=path, minimum=0.0),
        )
        if obj.hedge_notional_usd_max < obj.hedge_notional_usd_min:
            raise ConfigError(f"{path}: hedge_notional_usd_max must be >= hedge_notional_usd_min")
        return obj

    def size_for(self, main_notional_usd: float, side_price: float, seconds_left: float) -> float:
        """Hedge notional, or 0.0 when the extreme-skew trigger is not met."""
        if not self.enabled:
            return 0.0
        if side_price < self.trigger_side_price_gte:
            return 0.0
        if seconds_left > self.trigger_seconds_left_lte:
            return 0.0
        raw = main_notional_usd * (self.hedge_share_of_main_pct / 100.0)
        return round(min(self.hedge_notional_usd_max, max(self.hedge_notional_usd_min, raw)), 4)


@dataclass(frozen=True)
class StopLoss:
    enabled: bool = True
    stop_loss_pct_from_entry: float = 0.25

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "StopLoss":
        return cls(
            enabled=_bool(raw, "enabled", True, path=path),
            stop_loss_pct_from_entry=_num(
                raw, "stop_loss_pct_from_entry", 0.25, path=path, minimum=0.01, maximum=0.99
            ),
        )

    def price_for(self, entry_price: float) -> float:
        if not self.enabled:
            return 0.0
        return round(entry_price * (1.0 - self.stop_loss_pct_from_entry), 6)


@dataclass(frozen=True)
class RunnerOptions:
    """Loop mechanics that used to live only as argparse defaults."""

    poll_sec: float = 5.0
    entry_timeout_min: int = 60
    close_retry_max: int = 30
    close_retry_delay_sec: float = 2.0

    @classmethod
    def parse(cls, raw: Mapping[str, Any], path: str) -> "RunnerOptions":
        return cls(
            poll_sec=_num(raw, "poll_sec", 5.0, path=path, minimum=0.5, maximum=60.0),
            entry_timeout_min=_int(raw, "entry_timeout_min", 60, path=path, minimum=1, maximum=1440),
            close_retry_max=_int(raw, "close_retry_max", 30, path=path, minimum=1, maximum=200),
            close_retry_delay_sec=_num(raw, "close_retry_delay_sec", 2.0, path=path, minimum=0.1, maximum=60.0),
        )


@dataclass(frozen=True)
class Profile:
    """A fully-resolved profile: shared rules merged, everything validated."""

    name: str
    description: str
    signal: Signal
    sizing: Sizing
    hedge: Hedge
    stop_loss: StopLoss
    timing: SessionTiming
    safety: ExecutionSafety
    impulse: ImpulseFilter
    runner: RunnerOptions

    def as_dict(self) -> dict[str, Any]:
        from dataclasses import asdict

        return {
            "name": self.name,
            "description": self.description,
            "signal": asdict(self.signal),
            "sizing": asdict(self.sizing),
            "hedge": asdict(self.hedge),
            "stop_loss": asdict(self.stop_loss),
            "timing": asdict(self.timing),
            "safety": asdict(self.safety),
            "impulse": asdict(self.impulse),
            "runner": asdict(self.runner),
        }


@dataclass(frozen=True)
class ProfileSet:
    profiles: dict[str, Profile] = field(default_factory=dict)
    source_path: Optional[str] = None

    def get(self, name: str) -> Profile:
        try:
            return self.profiles[name]
        except KeyError:
            known = ", ".join(sorted(self.profiles)) or "<none>"
            raise ConfigError(f"unknown profile {name!r}; available profiles: {known}") from None

    @property
    def names(self) -> list[str]:
        return sorted(self.profiles)


def load_profiles(path: str | os.PathLike[str] | None = None) -> ProfileSet:
    """Read, merge and validate the profile file.

    ``shared_rules`` supplies defaults; each profile may override any subsection.
    """
    cfg_path = Path(path or os.environ.get("BTC5M_CONFIG") or DEFAULT_CONFIG_PATH)
    if not cfg_path.is_file():
        raise ConfigError(f"profile file not found: {cfg_path}")

    try:
        raw = yaml.safe_load(cfg_path.read_text(encoding="utf-8")) or {}
    except yaml.YAMLError as exc:
        raise ConfigError(f"{cfg_path}: invalid YAML: {exc}") from exc
    if not isinstance(raw, Mapping):
        raise ConfigError(f"{cfg_path}: top level must be a mapping")

    shared = raw.get("shared_rules") or {}
    if not isinstance(shared, Mapping):
        raise ConfigError(f"{cfg_path}: shared_rules must be a mapping")

    profiles_raw = raw.get("profiles") or {}
    if not isinstance(profiles_raw, Mapping) or not profiles_raw:
        raise ConfigError(f"{cfg_path}: at least one profile must be defined under 'profiles'")

    resolved: dict[str, Profile] = {}
    for name, body in profiles_raw.items():
        if not isinstance(body, Mapping):
            raise ConfigError(f"{cfg_path}: profiles.{name} must be a mapping")
        merged = _deep_merge(shared, body)
        base = f"profiles.{name}"
        timing = SessionTiming.parse(merged.get("session_timing") or {}, f"{base}.session_timing")
        hedge = Hedge.parse(merged.get("hedge") or {}, f"{base}.hedge")
        if hedge.enabled and hedge.trigger_seconds_left_lte < timing.min_entry_seconds_left:
            # Dead configuration: entries stop before the hedge window opens, so
            # the hedge could never fire. v1 was full of parameters like this.
            raise ConfigError(
                f"{base}: hedge.trigger_seconds_left_lte ({hedge.trigger_seconds_left_lte}) is "
                f"below session_timing.min_entry_seconds_left ({timing.min_entry_seconds_left}), "
                "so the hedge can never trigger. Raise the trigger or disable the hedge."
            )
        resolved[str(name)] = Profile(
            name=str(name),
            description=str(merged.get("description") or ""),
            signal=Signal.parse(merged.get("signal") or {}, f"{base}.signal"),
            sizing=Sizing.parse(merged.get("sizing") or {}, f"{base}.sizing"),
            hedge=hedge,
            stop_loss=StopLoss.parse(merged.get("stop_loss") or {}, f"{base}.stop_loss"),
            timing=timing,
            safety=ExecutionSafety.parse(merged.get("execution_safety") or {}, f"{base}.execution_safety"),
            impulse=ImpulseFilter.parse(merged.get("impulse_filter") or {}, f"{base}.impulse_filter"),
            runner=RunnerOptions.parse(merged.get("runner") or {}, f"{base}.runner"),
        )

    return ProfileSet(profiles=resolved, source_path=str(cfg_path))
