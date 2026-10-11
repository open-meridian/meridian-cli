"""The plugin's pages: the operations role's common pages
(spec/plugin-pages-follow-their-role), built on `meridian.roles.operations`,
the daily page first, each at Open and View:

- Breaks (/): `read_breaks`, the open breaks in the accounts the person may
  read, and each one in the page's own section.
- Reconciliations (/reconciliations): `read_reconciliations`.
- Opening balances (/opening): `read_opening_balances`; confirmed by the act
  `confirm_opening_balance`, checked by `check_opening_draft` without
  changing anything, so an agent may prepare it (contract v20): the SDK
  fills the page it is made on, this one, and a page opened from a prepared
  change reads it with `request.prepared()`.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import meridian
from meridian.roles import operations

TITLE = "Reference operations"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

#: What __main__.py gives the pages: the settings as last delivered, and when
#: the last reconciliation ran.
running: dict[str, object] = {"due_business_days": 3, "last_reconciliation": ""}

LEVELS = ["write", "read"]


@pages.page("/", "Breaks", levels=LEVELS, answers=operations.BreaksRead, name="read_breaks")
async def breaks(request: meridian.Request) -> meridian.Response:
    """The open breaks in the accounts the person may read, and how many are
    past the day they are due."""
    found = await request.plugin.list_breaks(states=["open"])
    mine = [each for each in found.breaks if request.caller.may_read(each.account_id)]
    due = timedelta(days=int(str(running["due_business_days"])))
    today = datetime.now(UTC).date()
    overdue = [
        each
        for each in mine
        if each.first_seen_date and date.fromisoformat(each.first_seen_date) + due < today
    ]
    said = operations.BreaksRead(open_breaks=len(mine), overdue_breaks=len(overdue))
    return pages.answer("breaks.html", said, breaks=mine)


@pages.page(
    "/reconciliations",
    "Reconciliations",
    levels=LEVELS,
    answers=operations.ReconciliationsRead,
    name="read_reconciliations",
)
async def reconciliations(request: meridian.Request) -> meridian.Response:
    """When the last reconciliation ran."""
    said = operations.ReconciliationsRead(
        last_reconciliation=str(running["last_reconciliation"])
    )
    return pages.answer("reconciliations.html", said)


@dataclass(frozen=True)
class Confirmation:
    """An account's opening balance, as of a day."""

    account_id: str
    as_of_date: date


@dataclass(frozen=True)
class Draft:
    """What a confirmation would record, checked and not recorded."""

    account_id: str
    as_of_date: date
    may_confirm: bool


@pages.page(
    "/opening",
    "Opening balances",
    levels=LEVELS,
    answers=operations.OpeningBalancesRead,
    name="read_opening_balances",
)
async def opening_balances(request: meridian.Request) -> meridian.Response:
    """How many accounts the person may write await an opening balance; and,
    opened from a prepared change, the change, to confirm or not."""
    said = operations.OpeningBalancesRead(awaiting_an_opening_balance=len(request.caller.write))
    prepared = await request.prepared()
    started = json.loads(prepared.arguments_json) if prepared is not None else {}
    return pages.answer("opening_balances.html", said, prepared=prepared, started=started)


@pages.route(
    "/opening/check",
    levels="write",
    methods=["POST"],
    params=Confirmation,
    answers=Draft,
    reads=True,
    name="check_opening_draft",
)
async def check_opening_draft(request: meridian.Request) -> meridian.Response:
    """Check an opening balance without recording it: what `confirm` would
    record, and whether this person may confirm it."""
    asked: Confirmation = request.params
    draft = Draft(
        asked.account_id, asked.as_of_date, request.caller.may_write(asked.account_id)
    )
    if not draft.may_confirm:
        pages.refuse("this person may not write the account", ("account_id", "not writable"))
    return pages.answer("opening_balances.html", draft)


@pages.route(
    "/opening/confirm",
    levels="write",
    methods=["POST"],
    params=Confirmation,
    name="confirm_opening_balance",
    checked_by="check_opening_draft",
    accounts=["account_id"],
)
async def confirm_opening_balance(request: meridian.Request) -> meridian.Response:
    """Confirm an account's opening balance as of a day: recorded in the
    book for the person (W9.1)."""
    asked: Confirmation = request.params
    if not request.caller.may_write(asked.account_id):
        pages.refuse("this person may not write the account", ("account_id", "not writable"))
    await request.plugin.record_opening_balance(
        account_id=asked.account_id,
        as_of_date=asked.as_of_date,
        sources=[meridian.OpeningSource(kind="custodian", as_of_date=asked.as_of_date)],
        idempotency_key=f"opening:{asked.account_id}:{asked.as_of_date.isoformat()}",
        acting_for=request.caller.header,
    )
    return pages.answer(
        "opening_balances.html",
        Draft(asked.account_id, asked.as_of_date, True),
    )
