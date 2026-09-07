"""Shared fixtures.

The whole suite runs offline: no network, no credentials, no trading repo.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from btc5m.config import load_profiles  # noqa: E402
from btc5m.models import BUCKET_SECONDS, Market, Quote, bucket_5m  # noqa: E402

# A bucket-aligned instant, so "seconds left" arithmetic is exact.
T0 = float(bucket_5m(1788800000) + BUCKET_SECONDS)


@pytest.fixture(scope="session")
def profiles():
    return load_profiles(ROOT / "config" / "btc_5m_profiles.yaml")


@pytest.fixture
def conservative(profiles):
    return profiles.get("conservative")


@pytest.fixture
def demo(profiles):
    return profiles.get("demo")


@pytest.fixture
def market() -> Market:
    return Market(
        slug="btc-updown-5m-test",
        up_token="up", down_token="down",
        end_ts=T0 + BUCKET_SECONDS,
    )


def make_quote(at: float, *, up_ask: float = 0.75, down_ask: float = 0.25,
               spread: float = 0.01, depth: float = 200.0) -> Quote:
    """A well-formed book: pass the guards unless a test says otherwise."""
    return Quote(
        ts=at,
        up_ask=up_ask, up_bid=round(up_ask - spread, 4), up_ask_notional=depth,
        down_ask=down_ask, down_bid=round(down_ask - spread, 4), down_ask_notional=depth,
    )
