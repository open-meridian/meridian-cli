"""The plugin's pages, as people see them through the deployment's dashboard.

Each page is a view function and a template. `pages` declares each where its
view is, with the levels it serves -- the dashboard's home opens a session by
Manage at `admin`, by Open at `write`, by View at `read` -- sends the list
when the plugin registers, and answers 403 to a session at any other level
before the view runs. So a page is declared, checked and rendered in one
place, and the dashboard's tab row and the plugin agree by construction.

- Setup (/setup), at `admin`: what this plugin is and what the deployment
  lets it do. Manage configures a plugin and sees no account's data, so this
  page shows none; `meridian.testing` holds it to that (tests/test_page.py).
- Accounts (/), at `write` and `read`: the accounts the person may read here,
  and under Open one action that writes for them, opening an empty holdings
  statement (/statement, at `write` alone), sent with `acting_for` so the
  sidecar decides whether they may.

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

import time
import uuid
from pathlib import Path

import meridian

TITLE = "Reference plugin"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

# The grid's columns; its rows are the person's accounts.
COLUMNS = [
    {"key": "account", "label": "Account", "type": "code"},
    {"key": "may", "label": "You may"},
]


@pages.page("/setup", "Setup", levels="admin")
def setup(request: meridian.Request) -> str:
    identity, grants = request.plugin.identity, request.plugin.grants
    return pages.render(
        "setup.html",
        instance=identity.instance_id,
        roles=identity.roles,
        publish=grants.publish,
        subscribe=grants.subscribe,
    )


@pages.page("/", "Accounts", levels=["write", "read"])
def accounts(request: meridian.Request) -> str:
    return _accounts(request)


@pages.route("/statement", levels="write", methods=["POST"])
async def statement(request: meridian.Request) -> str:
    # Sent for the person: the sidecar admits it only in a session opened by
    # Open, for an account they may write, and records it as theirs.
    try:
        opened = await request.plugin.record_holdings_statement(
            source="reference",
            external_statement_id=f"reference-{uuid.uuid4()}",
            as_of_date=time.strftime("%Y-%m-%d", time.gmtime()),
            read_at_ns=time.time_ns(),
            expected_rows=0,
            acting_for=request.caller.header,
        )
    except meridian.MeridianError as refused:
        return _accounts(request, f"Refused: {refused}", "bad")
    return _accounts(request, f"Opened statement {opened.statement_id} for you.", "good")


def _accounts(request: meridian.Request, notice: str = "", tone: str = "good") -> str:
    """Every account the person may read, and whether they may write it too:
    write includes read, so the read set holds them all. The template offers
    the action only under Open, whose session is the one that may write."""
    caller = request.caller
    rows = [
        {"account": account, "may": "read and write" if caller.may_write(account) else "read"}
        for account in sorted(caller.read | caller.write)
    ]
    return pages.render("accounts.html", notice=notice, tone=tone, columns=COLUMNS, rows=rows)
