"""The plugin's pages: connectivity's Trading page, built on
`meridian.roles.connectivity` and drawn by its template
(`meridian/roles/connectivity/trading.html`), each the same answer as its
tool.

- Trading (/, Manage): `read_trading`, the key, the environment, each account
  read-only or with trading enabled, and the placements working, held back or
  of unknown outcome; counts and codes only.
- "Enable trading" (/enable-trading): `enable_trading`, an act of the
  connectivity role's admin, checked by `check_enable_trading` without
  changing anything, so an agent may prepare it (contract v20).
"""

from __future__ import annotations

from typing import Any

import meridian
from meridian.roles import connectivity

TITLE = "Reference trading"
TEMPLATE = "meridian/roles/connectivity/trading.html"
ENABLE = "/enable-trading"

pages = meridian.Pages(TITLE)

#: What __main__.py gives the pages: the running Trader.
running: dict[str, Any] = {"trader": None}


def trading_read() -> connectivity.TradingRead:
    return connectivity.trading_read(running["trader"], [], "Sample data: the stand-in venue")


def _reached(request: meridian.Request) -> str:
    asked: connectivity.TradingAsked = request.params
    code = (asked.venue_account_code or "").strip()
    trader = running["trader"]
    if trader is None or code not in trader.reached:
        pages.refuse("the key reaches no account under this venue account code",
                     ("venue_account_code", "not reached"))  # fmt: skip
    return code


@pages.page(
    "/", "Trading", levels="admin", answers=connectivity.TradingRead, name="read_trading"
)
async def trading(request: meridian.Request) -> meridian.Response:
    """Connectivity's state at the venue: each account read-only or with trading
    enabled, and the placements working, held back or of unknown outcome."""
    may = request.caller.level_for("connectivity") == meridian.AccessLevel.ACCESS_LEVEL_ADMIN
    return pages.answer(TEMPLATE, trading_read(), may_enable=may, enable=ENABLE,
                        venue="the stand-in venue")  # fmt: skip


@pages.route(
    ENABLE,
    levels="admin",
    methods=["POST"],
    params=connectivity.TradingAsked,
    answers=connectivity.TradingEnabled,
    name="enable_trading",
    checked_by="check_enable_trading",
    description="Enable trading on an account the key reaches, by its venue account code.",
)
async def enable_trading(request: meridian.Request) -> meridian.Response:
    code = _reached(request)
    trader = running["trader"]
    if not trader.trading_on(code):
        await trader.enable(
            code, {"venue_account_code": code, "enabled_by": request.caller.subject}
        )
    return pages.answer(
        TEMPLATE, connectivity.TradingEnabled(code, f"Trading enabled on {code}.")
    )


@pages.route(
    ENABLE + "/check",
    levels="admin",
    methods=["POST"],
    params=connectivity.TradingAsked,
    answers=connectivity.TradingEnabled,
    reads=True,
    name="check_enable_trading",
    description="Check enable_trading without enabling anything.",
)
async def check_enable_trading(request: meridian.Request) -> meridian.Response:
    code = _reached(request)
    already = running["trader"].trading_on(code)
    said = f"Trading is enabled on {code} already." if already else f"Would enable {code}."
    return pages.answer(TEMPLATE, connectivity.TradingEnabled(code, said))
