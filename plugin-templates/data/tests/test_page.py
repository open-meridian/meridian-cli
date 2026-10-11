"""The pages, against a stand-in for the sidecar.

`meridian plugin check --run-tests` runs these, and so does the plugin's CI.
None needs a deployment. `PageClient` asks the pages as the sidecar forwards
a request, for a session opened by Manage (`admin`), Open (`write`) or View
(`read`), and calls each page's read tool as the deployment's MCP surface
would.

Replace these as you replace the pages: at least each page rendered at each
level, its common sections in the role's order, and each tool's answer.
"""

from __future__ import annotations

import meridian
from meridian.testing import PageClient

from reference_plugin.convert import Converter
from reference_plugin.page import pages, use
from reference_plugin.vendor import Vendor


class Sidecar:
    """Who the plugin was launched as: what its pages reach of it."""

    identity = meridian.Identity("reference-1", roles=("data",))
    grants = meridian.Grants()


def client() -> PageClient:
    return PageClient(pages, Sidecar())


def running(vendor: Vendor | None = None) -> Converter:
    converter = Converter(Sidecar(), vendor or Vendor())  # type: ignore[arg-type]
    use(converter)
    return converter


def in_order(page: str, *said: str) -> None:
    at = [page.find(each) for each in said]
    assert -1 not in at and at == sorted(at), dict(zip(said, at, strict=True))


def test_each_page_is_served_to_an_admin_of_the_plugin_alone() -> None:
    running()
    served = {(r.page.path, r.level): r.response.status for r in client().every_page()}
    assert served == {
        ("/", "admin"): 200,
        ("/", "write"): 403,
        ("/", "read"): 403,
        ("/datasets", "admin"): 200,
        ("/datasets", "write"): 403,
        ("/datasets", "read"): 403,
    }


def test_the_connection_page_draws_the_roles_sections_in_order() -> None:
    vendor = Vendor(calls=3)
    running(vendor)
    page = client().get("/", "admin").text
    in_order(
        page,
        "<h2>Key</h2>",
        "No key needed",
        "<h2>Limits and use</h2>",
        "60 a minute",
        "<h2>Stream</h2>",
        "Does not stream",
        "Reference vendor",
    )
    vendor.failed = "the vendor answered 503"
    assert "the vendor answered 503" in client().get("/", "admin").text


def test_an_agent_reads_the_connection_through_its_tool() -> None:
    running(Vendor(calls=3))
    answered = client().call_tool("read_connection")
    assert answered.outcome == "unchanged"
    assert answered.data["mode"] == "Live"
    assert answered.data["calls"] == 3
    assert answered.data["limit"] == "60 a minute"
    assert answered.data["sessions"] is None


def test_the_datasets_page_says_whether_each_dataset_is_licensed() -> None:
    converter = running()
    page = client().get("/datasets", "admin").text
    in_order(
        page,
        "No wants yet",
        "<h2>Served</h2>",
        "<code>reference-1:daily</code>",
        "Not licensed yet",
        "<h2>Wants</h2>",
        "<h2>Subjects</h2>",
    )
    converter.licensed = frozenset({"reference-1:daily"})
    converter.standing["WNT-1"] = ("LCL-1",)
    answered = client().call_tool("read_datasets")
    assert answered.data["served"][0]["licensed"] is True
    assert answered.data["served"][0]["data"] == "meridian.v1.Price, meridian.v1.Bar"
    assert answered.data["wants"] == [
        {
            "dataset": "reference-1:daily",
            "asked": "WNT-1",
            "subjects": 1,
            "recorded": 0,
            "declined": 0,
            "state": "Kept current",
        }
    ]
