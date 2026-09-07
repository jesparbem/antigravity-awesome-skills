"""Profile loading: the YAML must actually drive behaviour."""

from __future__ import annotations

import textwrap

import pytest

from btc5m.config import load_profiles
from btc5m.errors import ConfigError


def write(tmp_path, body: str):
    path = tmp_path / "profiles.yaml"
    path.write_text(textwrap.dedent(body), encoding="utf-8")
    return path


BASE = """
    shared_rules:
      session_timing: {min_entry_seconds_left: 90, max_entry_seconds_left: 150, exit_before_sec: 20}
      execution_safety: {skip_if_spread_gt: 0.03}
      signal: {threshold_price: 0.70}
    profiles:
      a:
        description: base
      b:
        execution_safety: {skip_if_spread_gt: 0.09}
        signal: {threshold_price: 0.80}
"""


def test_shared_rules_are_inherited(tmp_path):
    profiles = load_profiles(write(tmp_path, BASE))
    assert profiles.get("a").safety.skip_if_spread_gt == 0.03
    assert profiles.get("a").signal.threshold_price == 0.70


def test_profile_overrides_win_and_do_not_leak(tmp_path):
    profiles = load_profiles(write(tmp_path, BASE))
    assert profiles.get("b").safety.skip_if_spread_gt == 0.09
    assert profiles.get("b").signal.threshold_price == 0.80
    # The override must not mutate the shared defaults for its siblings.
    assert profiles.get("a").safety.skip_if_spread_gt == 0.03


def test_partial_override_keeps_sibling_keys(tmp_path):
    """A deep merge, not a replacement: 'b' overrides one key of session_timing."""
    profiles = load_profiles(write(tmp_path, """
        shared_rules:
          session_timing: {min_entry_seconds_left: 90, max_entry_seconds_left: 150, exit_before_sec: 20}
        profiles:
          b:
            session_timing: {max_entry_seconds_left: 200}
    """))
    timing = profiles.get("b").timing
    assert timing.max_entry_seconds_left == 200
    assert timing.min_entry_seconds_left == 90   # inherited, not reset to the default
    assert timing.exit_before_sec == 20


def test_shipped_profiles_load(profiles):
    assert {"conservative", "aggressive", "demo"} <= set(profiles.names)


def test_unknown_profile_lists_alternatives(profiles):
    with pytest.raises(ConfigError, match="conservative"):
        profiles.get("nope")


@pytest.mark.parametrize("body,message", [
    ("profiles:\n  a:\n    signal: {threshold_price: 1.5}\n", "above the maximum"),
    ("profiles:\n  a:\n    signal: {threshold_price: 'high'}\n", "expected a number"),
    ("profiles:\n  a:\n    hedge: {enabled: 'sure'}\n", "expected true/false"),
    ("profiles:\n  a:\n    impulse_filter: {source: kraken}\n", "is not one of"),
    ("shared_rules: {}\n", "at least one profile"),
])
def test_invalid_values_fail_loudly(tmp_path, body, message):
    with pytest.raises(ConfigError, match=message):
        load_profiles(write(tmp_path, body))


def test_incoherent_entry_window_is_rejected(tmp_path):
    with pytest.raises(ConfigError, match="max_entry_seconds_left"):
        load_profiles(write(tmp_path, """
            profiles:
              a:
                session_timing: {min_entry_seconds_left: 120, max_entry_seconds_left: 60}
        """))


def test_exit_before_sec_must_leave_room_to_trade(tmp_path):
    """exit_before_sec >= min_entry_seconds_left closes every entry immediately."""
    with pytest.raises(ConfigError, match="exit_before_sec"):
        load_profiles(write(tmp_path, """
            profiles:
              a:
                session_timing: {min_entry_seconds_left: 30, max_entry_seconds_left: 150, exit_before_sec: 60}
        """))


def test_missing_file_is_reported(tmp_path):
    with pytest.raises(ConfigError, match="not found"):
        load_profiles(tmp_path / "absent.yaml")


def test_malformed_yaml_is_reported(tmp_path):
    with pytest.raises(ConfigError, match="invalid YAML"):
        load_profiles(write(tmp_path, "profiles: [unclosed\n"))


def test_unreachable_hedge_is_rejected(tmp_path):
    """A hedge whose window closes before entries open is dead configuration."""
    with pytest.raises(ConfigError, match="can never trigger"):
        load_profiles(write(tmp_path, """
            profiles:
              a:
                session_timing: {min_entry_seconds_left: 90, max_entry_seconds_left: 150, exit_before_sec: 20}
                hedge: {enabled: true, trigger_seconds_left_lte: 45}
        """))


def test_disabled_hedge_may_have_any_window(tmp_path):
    profiles = load_profiles(write(tmp_path, """
        profiles:
          a:
            session_timing: {min_entry_seconds_left: 90, max_entry_seconds_left: 150, exit_before_sec: 20}
            hedge: {enabled: false, trigger_seconds_left_lte: 45}
    """))
    assert profiles.get("a").hedge.enabled is False


def test_shipped_hedges_are_all_reachable(profiles):
    for name in profiles.names:
        profile = profiles.get(name)
        if profile.hedge.enabled:
            assert profile.hedge.trigger_seconds_left_lte >= profile.timing.min_entry_seconds_left
