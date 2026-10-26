"""Connectivity's Trading page (0.30.0): connectivity's state at the venue,
each the same answer as its tool (`read_trading`) -- each credential set or
not, never a value; the environment, said plainly; each account the key
reaches by its venue account code, read-only or with trading enabled, the
order types its catalogue offers and whether its `ems` mapped it here; and
the placements working, held back by a pause or a stop, or of unknown
outcome. Counts and codes only: an admin page shows no account's data.
Drawn by `meridian/roles/connectivity/trading.html`; "Enable trading" on a
read-only account is the connectivity role's admin's (`enable_trading`,
checked by `check_enable_trading`).

From 0.32.0 the page and its acts are the SDK's to declare (Alpaca's and
Tradier's `trading_page.py` kept them nearly byte for byte): `trading_pages`
declares Trading at `/admin/connections` with "Enable trading" and its
check; `raw_records_page`, Raw records for a plugin holding custody or
connectivity; and for a plugin holding the data role as well as
connectivity, `connection_pages` declares one page for both (457): the data
role's Connection at `/`, `read_connection`, carrying connectivity's
sections (`ConnectionRead.trading`) and its acts, in place of a Trading page
of its own. Each takes the plugin's state as it stands (`TradingState`: its
`trader`, its venue adapter `venue`, its account `scope`, and for raw records
its `store` and kept `values`) and its words for the venue."""

from __future__ import annotations

import dataclasses
import json
from collections.abc import Awaitable, Callable, Sequence
from datetime import UTC, datetime
from typing import Any, Protocol

from ...pages import Pages
from ...plugin.v1 import operations_pb2 as ops
from .._trading import AccountTrading, Credential, TradingRead
from ..data import ConnectionRead

__all__ = [
    "CONNECTION",
    "ENABLE_TRADING",
    "RAW",
    "TRADING",
    "AccountTrading",
    "TradingAsked",
    "TradingEnabled",
    "TradingRead",
    "TradingState",
    "connection_pages",
    "raw_records_page",
    "trading_pages",
    "trading_read",
]

#: Trading, its "Enable trading" act, and a check's suffix.
TRADING = "/admin/connections"
ENABLE_TRADING = "/admin/connections/enable-trading"
_CHECK = "/check"
#: The one Connection of a plugin holding data and connectivity (457), and
#: its "Enable trading".
CONNECTION = "/"
CONNECTION_ENABLE_TRADING = "/enable-trading"
#: Raw records.
RAW = "/raw"


@dataclasses.dataclass(frozen=True)
class TradingAsked:
    """An account to enable trading on, by its venue account code."""

    venue_account_code: str


@dataclasses.dataclass(frozen=True)
class TradingEnabled:
    """What enabling trading came to, or would."""

    venue_account_code: str
    said: str = ""


class TradingState(Protocol):
    """The plugin's state as the pages read it: its `Trader` (None before
    the role is held), its venue adapter (`venue`, None until set up), its
    account scope (`scope`), and, for raw records, its raw store (`store`,
    a mapping of a key to the JSON kept) and the windows it keeps
    (`values`)."""

    trader: Any
    venue: Any
    scope: Any


def _shown(at: datetime | None) -> str:
    return at.astimezone(UTC).isoformat(timespec="seconds").replace("+00:00", "Z") if at else ""


def trading_read(
    trader: Any,
    credentials: Sequence[Credential],
    environment: str,
    updates: str = "",
    *,
    reached_environment: str = "",
    environment_setting: str = "",
    environment_to_save: str = "",
    latency: Any = None,
) -> TradingRead:
    """Trading's record from the plugin's `Trader` (None before its role is
    held): the accounts it reached, each one's trading and catalogue, and its
    counts; `credentials` and `environment` in the vendor's words, `updates`
    how its orders are heard. From 0.32.0 the environment the session
    reached and the setting's value give the connection's badge
    (`meridian.connection_badge`), a refused saved Environment is said
    (`environment_to_save`), and `latency` (the plugin's `roles.Latency`)
    each session's latency."""
    from ..._settings import connection_badge

    mapped = trader.mapped() if trader is not None else frozenset()
    shown = trader.shown if trader is not None else None
    reached: tuple[str, ...] = trader.reached if trader is not None else ()
    return TradingRead(
        credentials=list(credentials),
        environment=environment,
        accounts=[
            AccountTrading(
                venue_account_code=code,
                trading=trader is not None and trader.trading_on(code),
                order_types=[
                    ops.OrderType.Name(t).removeprefix("ORDER_TYPE_").lower()
                    for t, _ in (trader.offered(code) if trader is not None else ())
                ],
                mapped=code in mapped,
            )
            for code in reached
        ],
        working=shown.working if shown else 0,
        held=shown.held if shown else 0,
        unknown=shown.unknown if shown else 0,
        last_poll=_shown(shown.last_poll) if shown else "",
        updates=updates,
        error=shown.error if shown else "",
        reached_environment=reached_environment,
        badge=connection_badge(environment_setting, reached_environment)
        if reached_environment
        else "",
        environment_to_save=environment_to_save,
        session_latency=latency.session_latency() if latency is not None else [],
    )


class _Trading:
    """What the Trading helpers share: the plugin's state and words, its
    record read, and enabling trading on an account the key reaches."""

    def __init__(
        self,
        state: Callable[[], Any],
        *,
        venue: str,
        credentials: Callable[[], Sequence[Credential]],
        environment: Callable[[], str],
        environment_setting: Callable[[], str],
        updates: Callable[[], str],
        environment_to_save: Callable[[], str],
        latency: Callable[[], Any],
    ) -> None:
        self.state = state
        self.venue = venue
        self.credentials = credentials
        self.environment = environment
        self.environment_setting = environment_setting
        self.updates = updates
        self.environment_to_save = environment_to_save
        self.latency = latency

    def reached_environment(self) -> str:
        adapter = self.state().venue
        return str(getattr(adapter, "environment", "") or "") if adapter is not None else ""

    def read(self) -> TradingRead:
        return trading_read(
            self.state().trader,
            self.credentials(),
            self.environment(),
            self.updates(),
            reached_environment=self.reached_environment(),
            environment_setting=self.environment_setting(),
            environment_to_save=self.environment_to_save(),
            latency=self.latency(),
        )

    def reached(self) -> list[str]:
        trader = self.state().trader
        return list(trader.reached) if trader is not None else []

    @staticmethod
    def may_enable(request: Any) -> bool:
        from ...client import AccessLevel

        return bool(request.caller.level_for("connectivity") == AccessLevel.ACCESS_LEVEL_ADMIN)

    def asked(self, pages: Pages, request: Any) -> str:
        asked: TradingAsked = request.params
        code = (asked.venue_account_code or "").strip()
        if code not in self.reached():
            pages.refuse(
                "the key reaches no account under this venue account code",
                ("venue_account_code", "not reached"),
            )
        return code

    async def enable(self, pages: Pages, request: Any) -> str:
        code = self.asked(pages, request)
        trader = self.state().trader
        if trader is None:
            pages.refuse(
                "this instance holds no connectivity role", ("venue_account_code", "no role")
            )
        if not trader.trading_on(code):
            await trader.enable(
                code,
                {
                    "venue_account_code": code,
                    "enabled_by": request.caller.subject,
                    "environment": self.environment(),
                },
            )
        return f"Trading enabled on {code} ({self.environment()})."

    def check(self, pages: Pages, request: Any) -> TradingEnabled:
        code = self.asked(pages, request)
        trader = self.state().trader
        already = trader is not None and trader.trading_on(code)
        said = (
            f"Trading is enabled on {code} already."
            if already
            else f"Would enable trading on {code} ({self.environment()})."
        )
        return TradingEnabled(code, said)


def _nothing() -> str:
    return ""


def _none() -> Any:
    return None


def trading_pages(
    pages: Pages,
    state: Callable[[], Any],
    *,
    venue: str,
    credentials: Callable[[], Sequence[Credential]],
    environment: Callable[[], str],
    environment_setting: Callable[[], str],
    updates: Callable[[], str] = _nothing,
    updates_label: str = "",
    updates_none: str = "",
    environment_to_save: Callable[[], str] = _nothing,
    latency: Callable[[], Any] = _none,
    template: str = "meridian/roles/connectivity/trading.html",
) -> None:
    """Declare connectivity's Trading page on `pages` (0.32.0, from Alpaca's
    and Tradier's): Trading at `/admin/connections` (`read_trading`), at
    Manage, its "Enable trading" on a read-only account for the connectivity
    role's admin (`enable_trading`, checked by `check_enable_trading`): a new
    account is read-only until then. `state` gives the plugin's state
    (`TradingState`); `venue` names the venue; `credentials`, `environment`
    (said plainly: Paper or Live, or sample data) and `environment_setting`
    (the setting's value, for the badge) are read on each request;
    `updates` says how its orders are heard, under `updates_label` and
    `updates_none` (the template's "Orders read" and "Not yet" by default);
    `latency` gives the plugin's `roles.Latency`, for the Sessions section."""
    from ...pages import Request, Response

    held = _Trading(
        state,
        venue=venue,
        credentials=credentials,
        environment=environment,
        environment_setting=environment_setting,
        updates=updates,
        environment_to_save=environment_to_save,
        latency=latency,
    )
    said_how: dict[str, Any] = {"venue": venue}
    if updates_label:
        said_how["updates_label"] = updates_label
    if updates_none:
        said_how["updates_none"] = updates_none

    def drawn(request: Request, **more: Any) -> Response:
        answer: Response = pages.answer(
            template,
            held.read(),
            may_enable=held.may_enable(request),
            enable=ENABLE_TRADING,
            **said_how,
            **more,
        )
        return answer

    @pages.page(
        TRADING,
        "Trading",
        levels="admin",
        roles="connectivity",
        answers=TradingRead,
        name="read_trading",
        description=(
            f"Connectivity's state at {venue}: each credential, set or not (never a value); "
            "the environment, said plainly, or sample data; each account the key reaches by "
            "its venue account code, read-only or with trading enabled, the order types its "
            "catalogue offers and whether its ems mapped it here; how many placements are "
            "working, held back by a pause or a stop, or of unknown outcome; the last read of "
            "the orders and how its orders are heard; the environment the session reached and "
            "the connection's badge, Demo or Live; each session's latency. No account's data."
        ),
    )
    async def trading(request: Request) -> Response:
        return drawn(request)

    @pages.route(
        ENABLE_TRADING,
        levels="admin",
        roles="connectivity",
        methods=["POST"],
        params=TradingAsked,
        answers=TradingEnabled,
        name="enable_trading",
        checked_by="check_enable_trading",
        description=(
            "Enable trading on an account the key reaches, by its venue account code: a new "
            "account is read-only until the connectivity role's admin enables it. Its "
            "catalogue is published at once."
        ),
    )
    async def enable_trading(request: Request) -> Response:
        said = await held.enable(pages, request)
        if request.tool_name:
            enabled: Response = pages.answer(
                template, TradingEnabled(request.params.venue_account_code.strip(), said)
            )
            return enabled
        return drawn(request, notice=said)

    @pages.route(
        ENABLE_TRADING + _CHECK,
        levels="admin",
        roles="connectivity",
        methods=["POST"],
        params=TradingAsked,
        answers=TradingEnabled,
        reads=True,
        name="check_enable_trading",
        description=(
            "Check enable_trading without enabling anything: whether the key reaches the "
            "account, and whether trading is enabled on it already."
        ),
    )
    async def check_enable_trading(request: Request) -> Response:
        checked: Response = pages.answer(template, held.check(pages, request))
        return checked


def _tab_key(title: str) -> str:
    return title.strip().lower().replace(" ", "-")


def _connection_tabs(read: Any) -> list[str]:
    """The merged Connection's tab row (0.32.3): its sections' own titles --
    Key, Limits and use, Stream, Trading (connectivity's sections), and
    Sessions only once a session is measured -- one group drawn at a time,
    picked by `?tab=` (`key`, `limits-and-use`, `stream`, `trading`,
    `sessions`), Key where none is asked."""
    trading = getattr(read, "trading", None)
    measured = getattr(read, "session_latency", None) or (
        getattr(trading, "session_latency", None) if trading is not None else None
    )
    return ["Key", "Limits and use", "Stream", "Trading"] + (["Sessions"] if measured else [])


def connection_pages(
    pages: Pages,
    state: Callable[[], Any],
    connection: Callable[[Any], Awaitable[Any]],
    *,
    venue: str,
    credentials: Callable[[], Sequence[Credential]],
    environment: Callable[[], str],
    environment_setting: Callable[[], str],
    updates: Callable[[], str] = _nothing,
    environment_to_save: Callable[[], str] = _nothing,
    latency: Callable[[], Any] = _none,
    template: str = "meridian/roles/data/connection.html",
) -> None:
    """Declare one Connection for a plugin holding the data role and
    connectivity (0.32.0; 457), in place of data's Connection and
    connectivity's Trading apart: the data role's Connection at `/`
    (`read_connection`), at Manage, its record the plugin's own
    (`connection(request)`, a `data.ConnectionRead` or one extending it)
    carrying connectivity's Trading as `trading`, drawn as Connection's
    further sections ("Environment", "Accounts", "Placements"); and its
    "Enable trading" (`enable_trading`, checked by `check_enable_trading`)
    made on it. From 0.32.3 the page draws one group at a time under a tab
    row (`?tab=`), so each fits one screen; "Enable
    trading" answers on the Trading tab, and `read_connection` still answers
    every section. The rest as `trading_pages`."""
    from ...pages import Request, Response

    held = _Trading(
        state,
        venue=venue,
        credentials=credentials,
        environment=environment,
        environment_setting=environment_setting,
        updates=updates,
        environment_to_save=environment_to_save,
        latency=latency,
    )

    async def record(request: Request) -> Any:
        own = await connection(request)
        return dataclasses.replace(own, trading=held.read())

    async def drawn(request: Request, tab: str = "", **more: Any) -> Response:
        read = await record(request)
        tabs = _connection_tabs(read)
        asked = tab or request.query.get("tab", "")
        current = next((t for t in tabs if _tab_key(t) == _tab_key(asked)), tabs[0])
        answer: Response = pages.answer(
            template,
            read,
            may_enable=held.may_enable(request),
            enable=CONNECTION_ENABLE_TRADING,
            venue=venue,
            tabs=tabs,
            tab=current,
            **more,
        )
        return answer

    @pages.page(
        CONNECTION,
        "Connection",
        levels="admin",
        roles=["data", "connectivity"],
        answers=ConnectionRead,
        name="read_connection",
        description=(
            f"The connection to {venue} for both its roles: each credential, set or not (never "
            "a value); what the vendor serves the key, its limits and use, its stream and each "
            "session's latency; and for trading, the environment, said plainly, each account "
            "the key reaches by its venue account code, read-only or with trading enabled, the "
            "placements working, held back or of unknown outcome, and the connection's badge, "
            "Demo or Live. No account's data."
        ),
    )
    async def connection_page(request: Request) -> Response:
        return await drawn(request)

    @pages.route(
        CONNECTION_ENABLE_TRADING,
        levels="admin",
        roles="connectivity",
        methods=["POST"],
        params=TradingAsked,
        answers=TradingEnabled,
        name="enable_trading",
        checked_by="check_enable_trading",
        made_on=CONNECTION,
        description=(
            "Enable trading on an account the key reaches, by its venue account code: a new "
            "account is read-only until the connectivity role's admin enables it. Its "
            "catalogue is published at once."
        ),
    )
    async def enable_trading(request: Request) -> Response:
        said = await held.enable(pages, request)
        if request.tool_name:
            enabled: Response = pages.answer(
                template, TradingEnabled(request.params.venue_account_code.strip(), said)
            )
            return enabled
        return await drawn(request, tab="Trading", notice=said)

    @pages.route(
        CONNECTION_ENABLE_TRADING + _CHECK,
        levels="admin",
        roles="connectivity",
        methods=["POST"],
        params=TradingAsked,
        answers=TradingEnabled,
        reads=True,
        name="check_enable_trading",
        description=(
            "Check enable_trading without enabling anything: whether the key reaches the "
            "account, and whether trading is enabled on it already."
        ),
    )
    async def check_enable_trading(request: Request) -> Response:
        checked: Response = pages.answer(template, held.check(pages, request))
        return checked


def raw_records_page(
    pages: Pages,
    state: Callable[[], Any],
    *,
    kinds: Sequence[Any],
    why: str,
    template: str = "meridian/roles/custody/raw_records.html",
) -> None:
    """Declare Raw records (`/raw`) for a plugin holding custody or
    connectivity (0.32.0, from Alpaca's and Tradier's), at Open and View, no
    tool of its own (`why` says how an agent reads what they came to): each
    kind kept (`kinds`), its window and what becomes of it past it, and a raw
    record followed from a fact's or a row's reference (`?ref=`), shown to a
    person who may read an account mapped on this instance or linked by
    it."""
    from ...pages import Request

    def may_read_mapped(request: Request) -> bool:
        held = state()
        trader, scope = held.trader, held.scope
        if any(link.account_id in request.caller.read_for("custody") for link in scope.links):
            return True
        if trader is None or scope.venue_account_map is None:
            return False
        allowed = request.caller.read_for("connectivity")
        return any(
            mapping.account_id in allowed
            for mapping in scope.venue_account_map.mappings
            if mapping.connectivity_instance_id == trader.me
        )

    @pages.page(
        RAW,
        "Raw records",
        levels=["write", "read"],
        roles=["custody", "connectivity"],
        tool=False,
        why=why,
    )
    async def raw_records(request: Request) -> str:
        held = state()
        key = request.query.get("ref", "").strip()
        found: Any = None
        record: Any = None
        if key:
            finding = getattr(request.plugin, "find_record", None)
            found = finding(key.partition("#")[0]) if finding is not None else None
            store = getattr(held, "store", None)
            kept = store.get(key) if store is not None else None
            if kept is not None and may_read_mapped(request):
                record = json.dumps(kept, indent=1, sort_keys=True)
        drawn: str = pages.render(
            template,
            kinds=list(kinds),
            stored=list(getattr(request.plugin, "stored", None) or []),
            past=getattr(held, "values", {}),
            key=key,
            found=found,
            record=record,
        )
        return drawn
