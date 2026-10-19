"""A custody plugin's common part (spec/plugin-pages-follow-their-role,
"custody, a custody plugin"). Candidates until a second custody plugin needs
them (the spec's Q4), except what the role's duties name: statements, links,
raw records.

**First-use path.** Enter the credentials (Settings); connect (Connections);
have accounts granted (a deployment admin, from contract v20: a plugin links
only to the accounts granted to its instance, nothing by default); link
accounts (Account links); see statements (Statements).

**Settings**, in this order: the credentials; `poll_seconds()` ("Read every")
and `stale_after_hours()` ("Stale after") where the custodian is read on a
schedule; the vendor's own; `meridian.sample_data()`.

**Pages.** Manage: Connections (`read_connections`), then Account links
(`read_account_links`; linking with `link_account`). Open and View, the daily
page first: Statements (`read_statements`), History
(`read_account_activities`), Raw records (no tool of its own: a raw record is
followed from a row's reference, and restored with `restore_unit`).
Templates: `meridian/roles/custody/connections.html`, `account_links.html`,
`statements.html`, `history.html`, `raw_records.html`.

**Figures:** "Connections", "Accounts reached", "Last read" (`figures(...)`).

From contract v20 `read_accounts_for_linking` answers only the accounts the
deployment admin granted this instance, and a link naming another is refused
(`CommandRefused`, REFUSAL_REASON_ACCOUNT_NOT_LINKABLE): Account links says
"No accounts granted yet" until there are some.

**The pages themselves** (0.31.0; ruling 213: Alpaca's and Tradier's
custody_pages.py, kept between them nearly byte for byte, are the SDK's
now). `account_pages(pages, source, what=...)` declares a custody plugin's
Account links (`read_account_links`; linking with `link_account`, checked by
`check_link_account`), Statements (`read_statements`) and History
(`read_account_activities`), each the same answer as its tool, Statements
and History showing an account only to a person who may read the account it
is linked to. The plugin supplies a `CustodySource`: its vendor's name, the
word for its credential, the accounts it reaches, each one's name and the
type a new account is pre-filled with, the custodian, and each account's
statement and activities as its own sync holds them. Linking is the SDK's
(`LinkAsked`, the kit's account map's fields): an account the credential
does not reach refused, a new account created only by a deployment admin,
an account not granted refused before anything is sent.
"""

from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Protocol

from ..client import Setting
from ..figures import Figure
from ..plugin.v1 import operations_pb2 as _ops
from ._figures import Value, ordered

if TYPE_CHECKING:
    from ..pages import Pages
from .data import Credential

__all__ = [
    "ACCOUNT_LINKS",
    "HISTORY",
    "STATEMENTS",
    "CustodySource",
    "LinkAsked",
    "account_pages",
    "AccountLinksRead",
    "AccountRead",
    "ActivityRead",
    "ConnectionRead",
    "ConnectionsRead",
    "Credential",
    "ExternalAccountRead",
    "HistoryRead",
    "LinkRead",
    "StatementRead",
    "StatementsRead",
    "figures",
    "poll_seconds",
    "stale_after_hours",
]


def poll_seconds(
    *, default: int = 900, description: str = "", roles: tuple[str, ...] = ()
) -> Any:
    """ "Read every": how often the custodian is read, in seconds."""
    return Setting(
        "poll_seconds",
        int,
        label="Read every",
        default=default,
        unit="seconds",
        description=description or "How often to read the custodian.",
        roles=roles,
    )


def stale_after_hours(
    *, default: int = 36, description: str = "", roles: tuple[str, ...] = ()
) -> Any:
    """ "Stale after": how old the custodian's last sync of an account may be
    before it is reported stale, in hours."""
    return Setting(
        "stale_after_hours",
        int,
        label="Stale after",
        default=default,
        unit="hours",
        description=description
        or "How old the custodian's last sync of an account may be before it is stale.",
        roles=roles,
    )


@dataclass(frozen=True)
class ConnectionRead:
    """One connection ("Connections"): its name, its state (whose fix it is,
    as W2.1 says it), the accounts it reaches, and when it was last read."""

    connection: str
    state: str = ""
    accounts_reached: int = 0
    last_read: str = ""


@dataclass(frozen=True)
class ConnectionsRead:
    """Connections' common fields (`read_connections`'s answer): each
    credential, set or not, and each connection."""

    credentials: list[Credential] = field(default_factory=list)
    connections: list[ConnectionRead] = field(default_factory=list)


@dataclass(frozen=True)
class ExternalAccountRead:
    """One external account the plugin reports, as the kit's account map
    takes it: from SDK 0.29.0 with the source's own type for it
    (`account_type`), as the custodian names it -- "Margin", "Roth IRA" --
    which the map shows beside its name, so a person links the right one."""

    external_account_id: str
    name: str = ""
    detail: str = ""
    account_type: str = ""


@dataclass(frozen=True)
class AccountRead:
    """One account granted to this instance, as `read_accounts_for_linking`
    answers it."""

    account_id: str
    name: str = ""


@dataclass(frozen=True)
class LinkRead:
    """One link, as the SDK gives it (`AccountScope.links`)."""

    external_account_id: str
    account_id: str
    account_name: str = ""


@dataclass(frozen=True)
class AccountLinksRead:
    """Account links' common fields (`read_account_links`'s answer): the
    external accounts reported, the accounts granted, and the links."""

    external_accounts: list[ExternalAccountRead] = field(default_factory=list)
    accounts: list[AccountRead] = field(default_factory=list)
    links: list[LinkRead] = field(default_factory=list)


@dataclass(frozen=True)
class StatementRead:
    """One account's statement, for a person who may read it: the account,
    its custodian, the day it is as of, and its state (open, closed, or
    withheld with why)."""

    account: str
    custodian: str = ""
    as_of: str = ""
    state: str = ""


@dataclass(frozen=True)
class StatementsRead:
    """Statements' common fields (`read_statements`'s answer), cut to the
    accounts the person may read."""

    statements: list[StatementRead] = field(default_factory=list)


@dataclass(frozen=True)
class ActivityRead:
    """One activity on an account as the custodian reported it, in
    `meridian.CustodialActivity`'s words, each value as text: its kind, its
    dates, its instrument, its units and amount, and its description."""

    account: str
    external_activity_id: str = ""
    kind: str = ""
    trade_date: str = ""
    settlement_date: str = ""
    instrument_id: str = ""
    units: str = ""
    amount: str = ""
    description: str = ""


@dataclass(frozen=True)
class HistoryRead:
    """History's common fields (`read_account_activities`'s answer), cut to
    the accounts the person may read."""

    activities: list[ActivityRead] = field(default_factory=list)


def figures(
    *, connections: Value, accounts_reached: Value = None, last_read: Value = None
) -> list[Figure]:
    """The custody role's figures, in its order: "Connections" (a count, warn
    when any needs attention), "Accounts reached", "Last read" (error with
    the reason when the last read failed); each a plain value or a
    `meridian.Figure` carrying its state and why."""
    return ordered(
        [
            ("Connections", connections, None),
            ("Accounts reached", accounts_reached, None),
            ("Last read", last_read, None),
        ]
    )


# ── The pages (0.31.0; ruling 213) ─────────────────────────────────────────

ACCOUNT_LINKS = "/links"
STATEMENTS = "/statements"
HISTORY = "/history"
_CHECK = "/check"


@dataclass(frozen=True)
class LinkAsked:
    """A link asked for on Account links, in the kit's account map's fields
    (`intent`: link, create or unlink): the external account, and the
    account granted it is linked to, or a new account's name, custodian and
    type for a deployment admin to create and link in one step."""

    external_account_id: str
    intent: str = "link"
    account_id: str = ""
    new_account_name: str = ""
    new_account_custodian: str = ""
    new_account_type: str = ""


class CustodySource(Protocol):
    """What a custody plugin's pages read of its own sync: its vendor's words,
    the accounts its credential reaches, and each one's statement and
    activities. Read on every request, so it answers what the plugin holds
    now."""

    #: The vendor's name, as people know it (`Alpaca`).
    vendor: str
    #: The word for its credential (`key`, `token`).
    credential: str

    def reached(self) -> Sequence[str]:
        """The external accounts the credential reaches, by number."""
        ...

    def account(self, number: str) -> ExternalAccountRead:
        """One reached account as Account links shows it."""
        ...

    def custodian(self) -> str:
        """The custodian's name, as a statement and a new account name it."""
        ...

    def account_type(self, number: str) -> str:
        """The type a new account for `number` is pre-filled with; empty for
        none."""
        ...

    def statement(self, number: str) -> tuple[str, str] | None:
        """The account's last statement: the day it is as of and its state;
        None while it has none."""
        ...

    def activities(self, number: str) -> Sequence[tuple[Any, str]]:
        """The account's activities as its custodian reported them, the
        latest first: each a converted `meridian.CustodialActivity`-like
        record (external_activity_id, kind, trade_date, units, amount,
        description) and the instrument it resolved to."""
        ...

    def links(self) -> Sequence[Any]:
        """The links in force (`AccountScope.links`): external_account_id,
        account_id, account_name."""
        ...


def account_pages(
    pages: Pages,
    source: Callable[[], CustodySource],
    *,
    what: str,
    templates: tuple[str, str, str] = (
        "meridian/roles/custody/account_links.html",
        "meridian/roles/custody/statements.html",
        "meridian/roles/custody/history.html",
    ),
) -> None:
    """Declare custody's three pages on `pages`, each the same answer as its
    tool (spec/plugin-pages-follow-their-role): Account links at admin
    (`read_account_links`, `check_link_account`, `link_account`), Statements
    and History at write and read (`read_statements`,
    `read_account_activities`). `source` gives the plugin's source as it
    stands; `what` says what the plugin brings in, on Statements and
    History; `templates` are the role's (`meridian/roles/custody/<page>.html`)
    unless the plugin names its own, each extending the role's at its
    blocks."""
    from ..pages import Request, Response

    links_template, statements_template, history_template = templates

    def readable(request: Request) -> dict[str, str]:
        allowed = request.caller.read_for("custody")
        return {
            link.external_account_id: link.account_name or link.account_id
            for link in source().links()
            if link.account_id in allowed
        }

    async def links_read(request: Request) -> AccountLinksRead:
        held = source()
        granted = await request.plugin.read_accounts_for_linking(
            acting_for=request.caller.header
        )
        return AccountLinksRead(
            external_accounts=[held.account(number) for number in held.reached()],
            accounts=[AccountRead(each.account_id, each.name) for each in granted.accounts],
            links=[
                LinkRead(link.external_account_id, link.account_id, link.account_name)
                for link in held.links()
            ],
        )

    def asked(request: Request) -> LinkAsked:
        link: LinkAsked = request.params
        held = source()
        if link.external_account_id not in held.reached():
            pages.refuse(
                f"the {held.credential} reaches no such account",
                ("external_account_id", "not reached"),
            )
        return link

    @pages.page(
        ACCOUNT_LINKS,
        "Account links",
        levels="admin",
        roles="custody",
        answers=AccountLinksRead,
        name="read_account_links",
        description=(
            f"Each {source().vendor} account the {source().credential} reaches, by its number; "
            "the deployment's accounts a deployment admin granted this plugin to link to "
            "(nothing by default); and the links between them."
        ),
    )
    async def account_links(request: Request) -> Response:
        answer: Response = pages.answer(
            links_template, await links_read(request), action=ACCOUNT_LINKS
        )
        return answer

    @pages.route(
        ACCOUNT_LINKS + _CHECK,
        levels="admin",
        roles="custody",
        methods=["POST"],
        params=LinkAsked,
        answers=LinkRead,
        reads=True,
        name="check_link_account",
        description=(
            "Check link_account without linking: the external account is one the "
            f"{source().credential} reaches, and the account is one granted to this plugin, "
            "or a new one a deployment admin would create."
        ),
    )
    async def check_link_account(request: Request) -> Response:
        link = asked(request)
        if link.intent == "create" or link.new_account_name:
            if not request.caller.deployment_admin:
                pages.refuse(
                    "only a deployment admin creates an account",
                    ("new_account_name", "not allowed"),
                )
            answer: Response = pages.answer(
                links_template, LinkRead(link.external_account_id, "", link.new_account_name)
            )
            return answer
        granted = await request.plugin.read_accounts_for_linking(
            acting_for=request.caller.header
        )
        names = {each.account_id: each.name for each in granted.accounts}
        if link.account_id and link.account_id not in names:
            pages.refuse(
                "the account is not granted to this plugin", ("account_id", "not granted")
            )
        checked: Response = pages.answer(
            links_template,
            LinkRead(link.external_account_id, link.account_id, names.get(link.account_id, "")),
        )
        return checked

    @pages.route(
        ACCOUNT_LINKS,
        levels="admin",
        roles="custody",
        methods=["POST"],
        params=LinkAsked,
        name="link_account",
        checked_by="check_link_account",
        accounts=["account_id"],
        description=(
            f"Link a {source().vendor} account to one of the accounts granted to this plugin "
            "(W6.4), to a new account a deployment admin creates and links in one step, or, "
            "with neither, remove its link; one not granted is refused."
        ),
    )
    async def link_account(request: Request) -> Response:
        link = asked(request)
        held = source()
        creating = link.intent == "create" or bool(link.new_account_name)
        made = await request.plugin.link_external_account(
            external_account_id=link.external_account_id,
            account_id="" if creating or link.intent == "unlink" else link.account_id,
            new_account_name=link.new_account_name if creating else "",
            new_account_custodian=(link.new_account_custodian or held.custodian())
            if creating
            else "",
            new_account_type=(
                link.new_account_type or held.account_type(link.external_account_id)
            )
            if creating
            else "",
            acting_for=request.caller.header,
        )
        if request.tool_name:
            answered: Response = pages.answer(
                links_template, LinkRead(made.external_account_id, made.account_id)
            )
            return answered
        said = (
            f"Created {link.new_account_name} and linked {link.external_account_id} to it."
            if creating
            else f"Linked {link.external_account_id}."
        )
        shown: Response = pages.answer(
            links_template, await links_read(request), action=ACCOUNT_LINKS, notice=said
        )
        return shown

    @pages.page(
        STATEMENTS,
        "Statements",
        levels=["write", "read"],
        roles="custody",
        answers=StatementsRead,
        name="read_statements",
        description=(
            f"Each {source().vendor} account linked to one the person may read: its custodian, "
            f"the day its last statement is as of, and its state at {source().vendor}."
        ),
    )
    async def statements(request: Request) -> Response:
        held = source()
        shown = readable(request)
        rows = []
        for number in sorted(held.reached()):
            stated = held.statement(number)
            if number in shown and stated is not None:
                rows.append(StatementRead(shown[number], held.custodian(), *stated))
        answer: Response = pages.answer(
            statements_template, StatementsRead(statements=rows), what=what
        )
        return answer

    @pages.page(
        HISTORY,
        "History",
        levels=["write", "read"],
        roles="custody",
        answers=HistoryRead,
        name="read_account_activities",
        description=(
            f"Each {source().vendor} account's history as {source().vendor} reported it, where "
            "it is linked to one the person may read: each activity's kind, trade date, "
            "instrument, units, amount and description, the latest first."
        ),
    )
    async def history(request: Request) -> Response:
        held = source()
        shown = readable(request)
        rows = []
        for number in sorted(held.reached()):
            if number not in shown:
                continue
            for converted, instrument in held.activities(number):
                rows.append(
                    ActivityRead(
                        account=shown[number],
                        external_activity_id=converted.external_activity_id,
                        kind=_ops.ActivityKind.Name(converted.kind)
                        .removeprefix("ACTIVITY_KIND_")
                        .replace("_", " ")
                        .lower(),
                        trade_date=converted.trade_date,
                        instrument_id=instrument,
                        units="" if converted.units is None else str(converted.units),
                        amount="" if converted.amount is None else str(converted.amount.amount),
                        description=converted.description,
                    )
                )
        answer: Response = pages.answer(
            history_template, HistoryRead(activities=rows), what=what
        )
        return answer
