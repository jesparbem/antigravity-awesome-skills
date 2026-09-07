"""btc5m — BTC 5-minute Up/Down trading toolkit for Polymarket.

Two front-ends share one core:

* ``btc5m.tui``  — terminal console dashboard
* ``web.app``    — hardened web dashboard

The core is deliberately side-effect free where it matters: :mod:`btc5m.strategy`
and :mod:`btc5m.risk` are pure functions over immutable inputs, so every trading
decision is unit-testable without touching the network or an exchange.
"""

__version__ = "2.0.0"
