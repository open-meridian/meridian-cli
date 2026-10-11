"""The plugin's pages, as an admin of the plugin sees them through the
deployment's dashboard: the data role's common pages
(spec/plugin-pages-follow-their-role), built on `meridian.roles.data`.

A data plugin shows its connection and its datasets, never a price: prices
are the reading roles', and an admin configures data without reaching any
account. So both pages are at `admin`, under the dashboard's Manage, in the
role's order:

- Connection (/): "Key", "Limits and use" and "Stream", from
  `data.ConnectionRead`;
- Datasets (/datasets): "Served", "Wants" and "Subjects", from
  `data.DatasetsRead`, each dataset said "Licensed" or "Not licensed yet".

Each page declares the record it renders (`answers=`), so it is also a read
tool on the deployment's MCP surface, under the role's name:
`read_connection` and `read_datasets`. The templates extend the role's
(`meridian/roles/data/<page>.html`) and add the vendor's nuance only at its
blocks -- `vendor_notices`, `vendor_rows`, `vendor_columns`,
`vendor_sections` -- never renaming, reordering or dropping a common item.
"""

from __future__ import annotations

from pathlib import Path

import meridian
from meridian.roles import data
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


def _converter(request: meridian.Request) -> Converter:
    return running[0] if running else Converter(request.plugin)


def connection_read(converter: Converter) -> data.ConnectionRead:
    """Connection's record: the role's common fields."""
    vendor = converter.vendor
    return data.ConnectionRead(
        mode="Sample data" if converter.sample_data else "Live",
        # The stand-in vendor needs no key: "No key needed".
        credentials=[],
        entitled_at_vendor=f"{len(vendor.entitled)} symbols",
        limit=f"{vendor.limit_per_minute} a minute",
        calls=vendor.calls,
        last_error=vendor.failed,
        # It reads daily candles on request and keeps no stream open.
        sessions=None,
    )


def _mode(mode: int) -> str:
    return sidecar_pb2.ObservationMode.Name(mode).removeprefix("OBSERVATION_MODE_").lower()


def datasets_read(converter: Converter) -> data.DatasetsRead:
    """Datasets' record: the role's common fields."""
    instance = converter.plugin.identity.instance_id
    served = [
        data.Served(
            dataset=f"{instance}:{declared.key}",
            data=", ".join(declared.data_types),
            arrives=", ".join(_mode(int(mode)) for mode in declared.modes),
            day=declared.day_time_zone,
            default_terms=(
                "kept"
                if declared.licence_default is not None and declared.licence_default.kept
                else "served, not kept"
            ),
            entitled_at_vendor=f"{len(converter.vendor.entitled)} symbols",
            recorded=converter.recorded,
            declined=converter.declined,
            last_recorded=converter.last_recorded,
            licensed=f"{instance}:{declared.key}" in converter.licensed,
        )
        for declared in DECLARATION.catalogue
    ]
    wants = [
        data.Want(
            dataset=converter.dataset, asked=want, subjects=len(subjects), state="Kept current"
        )
        for want, subjects in converter.standing.items()
    ]
    return data.DatasetsRead(served=served, wants=wants)


@pages.page(
    "/", "Connection", levels="admin", answers=data.ConnectionRead, name="read_connection"
)
async def connection(request: meridian.Request) -> meridian.Response:
    """Whether the vendor answers, what this plugin asked of it, and the
    limit its terms set."""
    return pages.answer("connection.html", connection_read(_converter(request)))


@pages.page(
    "/datasets", "Datasets", levels="admin", answers=data.DatasetsRead, name="read_datasets"
)
async def datasets(request: meridian.Request) -> meridian.Response:
    """Each dataset this plugin serves the lake, whether it is licensed, and
    the standing wants it keeps current."""
    return pages.answer("datasets.html", datasets_read(_converter(request)))
