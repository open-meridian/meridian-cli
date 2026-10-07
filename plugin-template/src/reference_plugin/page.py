"""The plugin's pages, as people see them through the deployment's dashboard.

Each page is a view function and a template. `pages` declares each where its
view is, with the levels it serves -- the dashboard's home opens a session by
Manage at `admin`, by Open at `write`, by View at `read` -- sends the list
when the plugin registers, and answers 403 to a session at any other level
before the view runs. So a page is declared, checked and rendered in one
place, and the dashboard's tab row and the plugin agree by construction.

- Setup (/setup), at `admin`: what this plugin is and what the deployment
  lets it do, and the external accounts it reports, each linked to one of the
  deployment's accounts by the admin viewing it (/link, at `admin` alone).
  Manage configures a plugin and sees no account's data, so this page shows
  none -- an account's name is its identity, not its data -- and
  `meridian.testing` holds it to that (tests/test_page.py).
- Accounts (/), at `write` and `read`: the accounts the person may read here,
  and under Open one action that writes for them, opening an empty holdings
  statement for an external account the plugin has linked (/statement, at
  `write` alone), sent with `acting_for` so the sidecar decides whether they
  may. With nothing linked there is nothing to record for: the sidecar
  refuses a statement naming no linked external account, so the page says to
  link one first, and where.

Link, then record: a plugin reports the external accounts its connection
reaches (`report_external_accounts`, __main__.py), an admin of the plugin
links each to one of the deployment's accounts on its Manage page, and only
then is what it records for that external account taken, and for that
account. Which are linked is the plugin's account scope (`account_scope()`),
which it reads rather than keeps.

A route that changes something declares its inputs as one typed record
(`params=`), and the SDK derives from it a tool an agent the person
delegated to can call on the deployment's MCP surface, at the route's
levels: `open_statement` here. Its view answers with `pages.answer` -- the
page for a browser, the typed record for an agent -- and refuses an agent
with `pages.refuse`. `meridian plugin check` fails a changing route with no
record unless it says why it is not offered (`tool=False, why=...`).

Replace them with your plugin's own pages; keep declaring them this way.

A request reaches a view only through this plugin's sidecar, which verified
who is asking and says so in one `Meridian-Caller` header: `request.caller`,
with the level the session was opened at and the accounts cut to it. Nothing
here checks a signature, holds a key or keeps a session -- that is the point.

`pages.render` renders a Jinja2 template under templates/, every value
escaped, with `caller` and `level` in it. Each extends the kit's base
template, `meridian/base.html` (AGENTS.md says how): the kit the dashboard
serves on this plugin's own host, and the page's heading and tab row, which
the kit drops when the dashboard frames the page and draws its own. A page's
`status` and `head_actions` blocks hand the frame its status dot and its
buttons. The page draws only its content, and no colour of its own. Where
the kit is not served, it still works, unstyled: the table is inside
<om-grid> as plain HTML, and the action is a plain form.
"""

from __future__ import annotations

import asyncio
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

import meridian

TITLE = "Reference plugin"

# The external accounts this plugin's connection reaches, which __main__.py
# reports at start. A real plugin reports what its custodian's API lists;
# this one has no custodian, so it reaches one account, always.
REACHES = [
    meridian.ExternalAccount(external_account_id="reference-1", name="Reference account")
]

# What reporting them takes: a role that may publish the plugin's external
# accounts, `custody`. A plugin holding none reaches nothing it may report.
REPORTS = ".event.external-accounts"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

# The grid's columns; its rows are the person's accounts.
COLUMNS = [
    {"key": "account", "label": "Account", "type": "code"},
    {"key": "may", "label": "You may"},
]


def reports(plugin: meridian.Plugin) -> bool:
    """Whether the deployment lets this plugin report its external accounts."""
    return any(topic.endswith(REPORTS) for topic in plugin.grants.publish)


async def scope(plugin: meridian.Plugin) -> meridian.AccountScope:
    """The plugin's account scope and its links, as the sidecar has them now:
    the stream's first delivery, which comes at once."""
    delivered = plugin.account_scope()
    try:
        return await anext(delivered)
    finally:
        # Closed, so the stream to the sidecar ends here rather than whenever
        # the generator is collected.
        closing = getattr(delivered, "aclose", None)
        if closing is not None:
            await closing()


@pages.page("/setup", "Setup", levels="admin")
async def setup(request: meridian.Request) -> str:
    return await _setup(request)


async def _setup(request: meridian.Request, notice: str = "", tone: str = "good") -> str:
    """What the plugin is, and each external account it reaches with what it
    is linked to and the deployment's accounts to link it to, by name."""
    plugin = request.plugin
    identity, grants = plugin.identity, plugin.grants
    linking: list[dict[str, str]] = []
    accounts: list[dict[str, str]] = []
    now = meridian.AccountScope()
    if reports(plugin):
        now = await scope(plugin)
        for reached in REACHES:
            link = now.link_of(reached.external_account_id)
            linking.append(
                {
                    "external": reached.external_account_id,
                    "name": reached.name,
                    "linked": link.account_name or link.account_id if link else "",
                }
            )
        # Every account of the deployment's, by name, for the admin to link
        # to: an identity, not data, and read acting for them.
        try:
            read = await plugin.read_accounts_for_linking(acting_for=request.caller.header)
            accounts = [
                {"id": a.account_id, "name": a.name or a.account_id} for a in read.accounts
            ]
        except meridian.MeridianError as refused:
            notice, tone = notice or f"Refused: {refused}", "bad"
    return pages.render(
        "setup.html",
        instance=identity.instance_id,
        roles=identity.roles,
        publish=grants.publish,
        subscribe=grants.subscribe,
        reports=reports(plugin),
        linking=linking,
        accounts=accounts,
        # What the kit's om-account-map takes, from the same three.
        account_map={
            "external_accounts": [
                {"external_account_id": a.external_account_id, "name": a.name} for a in REACHES
            ],
            "accounts": [{"account_id": a["id"], "name": a["name"]} for a in accounts],
            "links": [
                {
                    "external_account_id": link.external_account_id,
                    "account_id": link.account_id,
                    "account_name": link.account_name,
                }
                for link in now.links
            ],
        },
        notice=notice,
        tone=tone,
    )


@dataclass(frozen=True)
class LinkAccount:
    """Which external account to link, and to which of the deployment's
    accounts; none, to remove its link. `intent` is what the kit's account
    map says the form was for: `link` or `unlink`."""

    external_account_id: str
    account_id: str = ""
    intent: str = "link"


@dataclass(frozen=True)
class Linked:
    """The link made."""

    external_account_id: str
    account_id: str


@pages.route(
    "/link",
    levels="admin",
    methods=["POST"],
    params=LinkAccount,
    name="link_account",
    description="Link an external account this plugin reaches to one of the deployment's.",
)
async def link(request: meridian.Request) -> meridian.Response | str:
    # Sent for the admin viewing Setup: the sidecar admits it only in a
    # session opened by Manage, and the deployment records it as theirs.
    asked = request.params
    try:
        made = await request.plugin.link_external_account(
            external_account_id=asked.external_account_id,
            account_id=asked.account_id,
            acting_for=request.caller.header,
        )
    except meridian.MeridianError as refused:
        if request.tool_name:
            pages.refuse(f"Refused: {refused}", reason="refused")
        return await _setup(request, f"Refused: {refused}", "bad")
    # The link reaches the account scope a moment after it is made.
    for _ in range(10):
        held = (await scope(request.plugin)).link_of(asked.external_account_id)
        if held is not None and held.account_id == made.account_id:
            break
        await asyncio.sleep(0.5)
    if request.tool_name:
        return pages.answer("setup.html", Linked(asked.external_account_id, made.account_id))
    if not made.account_id:
        return await _setup(request, f"Unlinked {asked.external_account_id}.")
    return await _setup(request, f"Linked {asked.external_account_id}.")


@pages.page("/", "Accounts", levels=["write", "read"])
async def accounts(request: meridian.Request) -> str:
    return await _accounts(request)


def _for(
    now: meridian.AccountScope, caller: meridian.Caller
) -> meridian.LinkedExternalAccount | None:
    """The linked external account a statement is opened for: one linked to
    an account the person may write, or, with none, any linked one, which
    the sidecar then refuses for them, saying why."""
    mine = [link for link in now.links if caller.may_write(link.account_id)]
    return next(iter(mine or now.links), None)


#: What the page says, and an agent is told, while nothing is linked.
LINK_FIRST = (
    "Link an account first: a statement is for an external account this plugin "
    "has linked, and none is. An admin of the plugin links them on its Setup "
    "page, under Manage."
)


@dataclass(frozen=True)
class OpenStatement:
    """What opening a statement takes: nothing."""


@dataclass(frozen=True)
class Opened:
    """The statement opened."""

    statement_id: str


@pages.route(
    "/statement",
    levels="write",
    methods=["POST"],
    params=OpenStatement,
    name="open_statement",
    description="Open an empty holdings statement, for the person.",
)
async def statement(request: meridian.Request) -> meridian.Response | str:
    # For a linked external account, which names the account it is for. The
    # sidecar admits it only in a session opened by Open, for an account the
    # person may write, and records it as theirs.
    linked = _for(await scope(request.plugin), request.caller)
    if linked is None:
        if request.tool_name:
            pages.refuse(LINK_FIRST, reason="not_linked")
        return await _accounts(request, LINK_FIRST, "warn")
    try:
        opened = await request.plugin.record_holdings_statement(
            source="reference",
            external_statement_id=f"reference-{uuid.uuid4()}",
            external_account_id=linked.external_account_id,
            as_of_date=time.strftime("%Y-%m-%d", time.gmtime()),
            read_at_ns=time.time_ns(),
            expected_rows=0,
            acting_for=request.caller.header,
        )
    except meridian.MeridianError as refused:
        if request.tool_name:
            pages.refuse(f"Refused: {refused}", reason="refused")
        return await _accounts(request, f"Refused: {refused}", "bad")
    if request.tool_name:
        return pages.answer("accounts.html", Opened(opened.statement_id))
    return await _accounts(request, f"Opened statement {opened.statement_id} for you.", "good")


async def _accounts(request: meridian.Request, notice: str = "", tone: str = "good") -> str:
    """Every account the person may read, and whether they may write it too:
    write includes read, so the read set holds them all. The template offers
    the action only under Open, whose session is the one that may write, and
    only for a linked external account; with none, it says to link one."""
    caller = request.caller
    rows = [
        {"account": account, "may": "read and write" if caller.may_write(account) else "read"}
        for account in sorted(caller.read | caller.write)
    ]
    linked = None
    if caller.level == meridian.AccessLevel.ACCESS_LEVEL_WRITE:
        linked = _for(await scope(request.plugin), caller)
        if linked is None and not notice:
            notice, tone = LINK_FIRST, "warn"
    return pages.render(
        "accounts.html", notice=notice, tone=tone, columns=COLUMNS, rows=rows, linked=linked
    )
