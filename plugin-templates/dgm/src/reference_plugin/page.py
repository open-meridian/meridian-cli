"""The plugin's pages, as an admin of the plugin sees them through the
deployment's dashboard.

A `dgm` shows its connection and its datasets, never a price: prices are the
reading roles', and an admin configures data without reaching any account.
So both pages are at `admin`, under the dashboard's Manage:

- Connection (/): whether the vendor answers, what this plugin asked of it,
  and the limit its terms set.
- Datasets (/datasets): each dataset it serves the lake, as its catalogue
  declares it, the standing wants it keeps current, and what the vendor
  serves this plugin's key.

Each page declares the typed data it renders (`answers=`), so it is also a
read tool on the deployment's MCP surface, answering that data to an agent
the admin delegated to: `read_connection` and `read_datasets`. The view
answers with `pages.answer`: the page for a browser, the record for an agent.

Replace them with your plugin's own pages; keep declaring them this way. See
the default template's page.py for what a page may do besides: a changing
route as a tool, a refusal, the kit's components.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import meridian
from meridian.v1 import sidecar_pb2

from .convert import Converter
from .declaration import DECLARATION

TITLE = "Reference prices"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

#: The running plugin's conversion, which __main__.py gives the pages.
running: list[Converter] = []


def use(converter: Converter) -> None:
    """The conversion the pages show: the one the plugin runs."""
    running[:] = [converter]


@dataclass(frozen=True)
class Connection:
    """The vendor, whether it answers, what it was asked, and its limit."""

    vendor: str
    healthy: bool
    detail: str
    calls: int
    limit_per_minute: int


@dataclass(frozen=True)
class Dataset:
    """One dataset it serves the lake: its name in the deployment, what it
    fills, how its rows arrive, and the standing wants it keeps current."""

    dataset: str
    vendor: str
    data_types: tuple[str, ...]
    modes: tuple[str, ...]
    standing_wants: int


@dataclass(frozen=True)
class Datasets:
    """Each dataset, and the symbols the vendor serves this plugin's key."""

    datasets: tuple[Dataset, ...]
    entitled: tuple[str, ...]


#: The Datasets grid's columns; its rows are the datasets.
COLUMNS = [
    {"key": "dataset", "label": "Dataset", "type": "code"},
    {"key": "vendor", "label": "Vendor"},
    {"key": "fills", "label": "Fills"},
    {"key": "modes", "label": "Arrives by"},
    {"key": "standing", "label": "Standing wants", "type": "number"},
]


def _converter(request: meridian.Request) -> Converter:
    return running[0] if running else Converter(request.plugin)


@pages.page("/", "Connection", levels="admin", answers=Connection, name="read_connection")
async def connection(request: meridian.Request) -> meridian.Response:
    """Whether the vendor answers, what this plugin asked of it, and the
    limit its terms set."""
    vendor = _converter(request).vendor
    said = Connection(
        vendor=DECLARATION.catalogue[0].vendor,
        healthy=not vendor.failed,
        detail=vendor.failed or "Answering.",
        calls=vendor.calls,
        limit_per_minute=vendor.limit_per_minute,
    )
    return pages.answer("connection.html", said)


@pages.page("/datasets", "Datasets", levels="admin", answers=Datasets, name="read_datasets")
async def datasets(request: meridian.Request) -> meridian.Response:
    """Each dataset this plugin serves the lake, the standing wants it keeps
    current, and what the vendor serves its key."""
    converter = _converter(request)
    instance = request.plugin.identity.instance_id
    said = Datasets(
        datasets=tuple(
            Dataset(
                dataset=f"{instance}:{declared.key}",
                vendor=declared.vendor,
                data_types=tuple(declared.data_types),
                modes=tuple(
                    sidecar_pb2.ObservationMode.Name(int(mode))
                    .removeprefix("OBSERVATION_MODE_")
                    .lower()
                    for mode in declared.modes
                ),
                standing_wants=len(converter.standing),
            )
            for declared in DECLARATION.catalogue
        ),
        entitled=converter.vendor.entitled,
    )
    rows = [
        {
            "dataset": row.dataset,
            "vendor": row.vendor,
            "fills": ", ".join(row.data_types),
            "modes": ", ".join(row.modes),
            "standing": row.standing_wants,
        }
        for row in said.datasets
    ]
    return pages.answer("datasets.html", said, columns=COLUMNS, rows=rows)
