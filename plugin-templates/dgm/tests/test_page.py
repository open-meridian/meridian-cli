"""The pages, against a stand-in for the sidecar.

`meridian plugin check --run-tests` runs these, and so does the plugin's CI.
None needs a deployment. `PageClient` asks the pages as the sidecar forwards
a request, for a session opened by Manage (`admin`), Open (`write`) or View
(`read`), and calls each page's read tool as the deployment's MCP surface
would.

Replace these as you replace the pages: at least each page rendered at each
level, and each tool's answer.
"""

from __future__ import annotations

import meridian
from meridian.testing import PageClient

from reference_plugin.convert import Converter
from reference_plugin.page import pages, use
from reference_plugin.vendor import Vendor


class Sidecar:
    """Who the plugin was launched as: what its pages reach of it."""

    identity = meridian.Identity("reference-1", roles=("dgm",))
    grants = meridian.Grants()


def client() -> PageClient:
    return PageClient(pages, Sidecar())


def running(vendor: Vendor | None = None) -> Converter:
    converter = Converter(Sidecar(), vendor or Vendor())  # type: ignore[arg-type]
    use(converter)
    return converter


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


def test_the_connection_page_says_whether_the_vendor_answers() -> None:
    vendor = Vendor(calls=3)
    running(vendor)
    page = client().get("/", "admin").text
    assert "Reference vendor" in page and "<dd>3</dd>" in page and "60 a minute" in page
    vendor.failed = "the vendor answered 503"
    assert "the vendor answered 503" in client().get("/", "admin").text


def test_an_agent_reads_the_connection_through_its_tool() -> None:
    running(Vendor(calls=3))
    answered = client().call_tool("read_connection")
    assert answered.outcome == "unchanged"
    assert answered.data == {
        "vendor": "Reference vendor",
        "healthy": True,
        "detail": "Answering.",
        "calls": 3,
        "limit_per_minute": 60,
    }


def test_the_datasets_page_shows_the_catalogue_and_the_standing_wants() -> None:
    converter = running()
    converter.standing["WNT-1"] = ("LCL-1",)
    page = client().get("/datasets", "admin").text
    assert "<code>reference-1:daily</code>" in page and "meridian.v1.Price" in page
    answered = client().call_tool("read_datasets")
    assert answered.data["datasets"] == [
        {
            "dataset": "reference-1:daily",
            "vendor": "Reference vendor",
            "data_types": ["meridian.v1.Price", "meridian.v1.Bar"],
            "modes": ["pull"],
            "standing_wants": 1,
        }
    ]
    assert "BTC-USD" in answered.data["entitled"]
