"""The plugin's pages, as people see them through the deployment's dashboard.

- Closes (/), at `write` and `read`: the positions in the accounts the person
  may read, at the last day's close, each with the close the lake holds, the
  dataset it came from, the value it makes and the change over the week.
  Computed and shown, never recorded. It declares the typed data it renders
  (`answers=`), so it is also a read tool on the deployment's MCP surface,
  `read_report`, answering the same rows to an agent the person delegated
  to, at the same levels: the view answers with `pages.answer`, the page for
  a browser and the record for an agent.
- Datasets (/datasets), at `admin`: the datasets this plugin is entitled to
  read, and what the lake has recorded for it since it started. Manage
  configures a plugin and sees no account's data, so this page shows none.

Replace them with your plugin's own pages; keep declaring them this way.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import meridian

from .report import Report, Row

TITLE = "Reference report"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

#: The running plugin's report, which __main__.py gives the pages.
running: list[Report] = []


def use(report: Report) -> None:
    """The report the pages show: the one the plugin runs."""
    running[:] = [report]


#: The Closes grid's columns; its rows are the positions.
COLUMNS = [
    {"key": "account", "label": "Account", "type": "code"},
    {"key": "instrument", "label": "Instrument", "type": "code"},
    {"key": "quantity", "label": "Quantity", "type": "decimal"},
    {"key": "close", "label": "Close", "type": "decimal"},
    {"key": "currency", "label": "In"},
    {"key": "value", "label": "Value", "type": "decimal"},
    {"key": "week", "label": "Over the week", "type": "decimal"},
    {"key": "dataset", "label": "From"},
]


@dataclass(frozen=True)
class AtTheClose:
    """What Closes shows: the business date, and each position in the
    accounts the person may read at its close."""

    business_date: date
    rows: tuple[Row, ...]


def last_close() -> date:
    """The last day whose close has passed: yesterday, in UTC."""
    return datetime.now(UTC).date() - timedelta(days=1)


def _report(request: meridian.Request) -> Report:
    return running[0] if running else Report(request.plugin)


def _text(value: object) -> str:
    return "" if value is None else str(value)


@pages.page("/", "Closes", levels=["write", "read"], answers=AtTheClose, name="read_report")
async def closes(request: meridian.Request) -> meridian.Response:
    """The positions in the accounts the person may read at the last day's
    close, each valued at the close the lake holds: computed, never recorded."""
    day = last_close()
    caller = request.caller
    said = AtTheClose(
        business_date=day,
        rows=tuple(
            row
            for row in await _report(request).rows(day)
            # The sidecar answers the plugin's whole account scope: shown,
            # only the accounts this person may read.
            if caller.may_read(row.account)
        ),
    )
    rows = [
        {
            "account": row.account,
            "instrument": row.instrument,
            "quantity": str(row.quantity),
            "close": _text(row.close),
            "currency": row.currency,
            "value": _text(row.value),
            "week": _text(row.week),
            "dataset": row.dataset,
        }
        for row in said.rows
    ]
    return pages.answer("closes.html", said, day=day.isoformat(), columns=COLUMNS, rows=rows)


@pages.page("/datasets", "Datasets", levels="admin")
async def datasets(request: meridian.Request) -> str:
    report = _report(request)
    read = await report.datasets()
    return pages.render("datasets.html", datasets=read, report=report)
