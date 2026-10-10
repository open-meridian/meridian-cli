"""The plugin's pages, as people see them through the deployment's dashboard.

- Closes (/), at `write` and `read`: the positions in the accounts the person
  may read, at the last day's close, each with the close the lake holds, the
  dataset it came from, the value it makes and the change over the week.
  Computed and shown, never recorded.
- Datasets (/datasets), at `admin`: the datasets this plugin is entitled to
  read, and what the lake has recorded for it since it started. Manage
  configures a plugin and sees no account's data, so this page shows none.

A page that declares the typed data it renders (`answers=`, with `name=`)
is also a read tool on the deployment's MCP surface, answering that data to
an agent the person delegated to; the `dgm` template's pages show how.

Replace them with your plugin's own pages; keep declaring them this way.
"""

from __future__ import annotations

from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import meridian

from .report import Report

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


def last_close() -> date:
    """The last day whose close has passed: yesterday, in UTC."""
    return datetime.now(UTC).date() - timedelta(days=1)


def _report(request: meridian.Request) -> Report:
    return running[0] if running else Report(request.plugin)


def _text(value: object) -> str:
    return "" if value is None else str(value)


@pages.page("/", "Closes", levels=["write", "read"])
async def closes(request: meridian.Request) -> str:
    day = last_close()
    caller = request.caller
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
        for row in await _report(request).rows(day)
        # The sidecar answers the plugin's whole account scope: shown, only
        # the accounts this person may read.
        if caller.may_read(row.account)
    ]
    return pages.render("closes.html", day=day.isoformat(), columns=COLUMNS, rows=rows)


@pages.page("/datasets", "Datasets", levels="admin")
async def datasets(request: meridian.Request) -> str:
    report = _report(request)
    read = await report.datasets()
    return pages.render("datasets.html", datasets=read, report=report)
