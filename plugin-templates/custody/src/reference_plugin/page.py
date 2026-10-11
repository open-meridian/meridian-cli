"""The plugin's pages: the custody role's common pages
(spec/plugin-pages-follow-their-role), built on `meridian.roles.custody`.

- Connections (/, Manage): `read_connections`.
- Account links (/links, Manage): `read_account_links`; linking, the act
  `link_account`, posted from the kit's account map. From contract v20 it
  links only to the accounts the deployment admin granted this instance,
  which `read_accounts_for_linking` answers.
- Statements (/statements, Open and View): `read_statements`, cut to the
  accounts the person may read.

Each template extends the role's (`meridian/roles/custody/<page>.html`) and
adds the custodian's nuance only at its blocks.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import meridian
from meridian.roles import custody

from .custodian import Custodian

TITLE = "Reference custodian"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

#: The running plugin's custodian, whether its key is set, and its account
#: scope with its links, as __main__.py last gave them the pages.
running: dict[str, object] = {}


def use(
    custodian: Custodian, *, key_set: bool, scope: meridian.AccountScope | None = None
) -> None:
    running.update(custodian=custodian, key_set=key_set, scope=scope or meridian.AccountScope())


def _links() -> tuple[meridian.LinkedExternalAccount, ...]:
    scope = running.get("scope")
    return scope.links if isinstance(scope, meridian.AccountScope) else ()


def _custodian() -> Custodian:
    found = running.get("custodian")
    return found if isinstance(found, Custodian) else Custodian()


@pages.page(
    "/", "Connections", levels="admin", answers=custody.ConnectionsRead, name="read_connections"
)
async def connections(request: meridian.Request) -> meridian.Response:
    """Each connection to the custodian: its state, the accounts it reaches,
    and when it was last read."""
    custodian = _custodian()
    said = custody.ConnectionsRead(
        credentials=[custody.Credential("API key", bool(running.get("key_set")))],
        connections=[
            custody.ConnectionRead(
                connection=custodian.connection,
                state=custodian.state,
                accounts_reached=len(custodian.accounts),
                last_read=custodian.last_read,
            )
        ],
    )
    return pages.answer("connections.html", said)


@pages.page(
    "/links",
    "Account links",
    levels="admin",
    answers=custody.AccountLinksRead,
    name="read_account_links",
)
async def account_links(request: meridian.Request) -> meridian.Response:
    """The external accounts reported beside the accounts granted to this
    plugin, and the links between them."""
    return pages.answer("account_links.html", await _links_read(request), action="/links")


async def _links_read(request: meridian.Request) -> custody.AccountLinksRead:
    granted = await request.plugin.read_accounts_for_linking(acting_for=request.caller.header)
    return custody.AccountLinksRead(
        external_accounts=[
            custody.ExternalAccountRead(each.external_account_id, each.name, each.detail)
            for each in _custodian().accounts
        ],
        accounts=[custody.AccountRead(each.account_id, each.name) for each in granted.accounts],
        links=[
            custody.LinkRead(link.external_account_id, link.account_id, link.account_name)
            for link in _links()
        ],
    )


@dataclass(frozen=True)
class Link:
    """One external account linked to one of the accounts granted."""

    external_account_id: str
    account_id: str


@pages.route("/links", levels="admin", methods=["POST"], params=Link, name="link_account")
async def link_account(request: meridian.Request) -> meridian.Response:
    """Link one external account to one of the accounts granted to this
    plugin (W6.4); one not granted is refused."""
    link: Link = request.params
    made = await request.plugin.link_external_account(
        external_account_id=link.external_account_id,
        account_id=link.account_id,
        acting_for=request.caller.header,
    )
    if request.tool_name:
        return pages.answer(
            "account_links.html", custody.LinkRead(made.external_account_id, made.account_id)
        )
    return pages.answer("account_links.html", await _links_read(request), action="/links")


@pages.page(
    "/statements",
    "Statements",
    levels=["write", "read"],
    answers=custody.StatementsRead,
    name="read_statements",
)
async def statements(request: meridian.Request) -> meridian.Response:
    """A row per account the person may read: its custodian, the day its
    statement is as of, and its state."""
    linked = {link.external_account_id: link for link in _links()}
    said = custody.StatementsRead(
        statements=[
            custody.StatementRead(
                account=linked[each.external_account_id].account_name or each.name,
                custodian="Reference custodian",
                as_of=each.as_of,
                state="Open",
            )
            for each in _custodian().accounts
            if each.external_account_id in linked
            and request.caller.may_read(linked[each.external_account_id].account_id)
        ]
    )
    return pages.answer(
        "statements.html",
        said,
        what="This plugin brings a custodian's accounts into this deployment",
    )
