"""The pages, against a stand-in for the sidecar.

`meridian plugin check --run-tests` runs these, and so does the plugin's CI.
None needs a deployment. `Sidecar` is the SDK's own operations with the
transport replaced, answering the book's and the lake's reads here.
`PageClient` asks the pages as the sidecar forwards a request, for a session
opened by Manage (`admin`), Open (`write`) or View (`read`), with the
person's accounts cut to it.

Replace these as you replace the pages: at least each page rendered at each
level, no account data under Manage, and what each figure is computed from.
"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from decimal import Decimal
from typing import Any, cast

import meridian
from meridian.operations import Operations
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.testing import PageClient

from reference_plugin.page import pages, use
from reference_plugin.report import Report


def wire(text: str) -> ops.Decimal:
    """A positive decimal as the wire carries it: its digits and its scale."""
    amount = Decimal(text)
    scale = max(0, -int(amount.as_tuple().exponent))
    return ops.Decimal(low=int(amount.scaleb(scale)), scale=scale)


def meta(subject: str, day: int = 0) -> ops.ObservationMeta:
    return ops.ObservationMeta(
        subjects=[ops.SubjectRef(entity_id=subject)],
        source=ops.Source(dataset="dgm-1:daily"),
        valid_from_ns=day,
    )


class _Named:
    """Each operation, named for itself, so what was read says which it was."""

    def __getattr__(self, name: str) -> str:
        return name


class Sidecar(Operations):
    """The book holding LCL-1 in ACC-1 and ACC-9, and the lake its close and
    two daily bars, answered here rather than by a sidecar."""

    identity = meridian.Identity("reference-1", roles=("reporting",))
    grants = meridian.Grants()

    def __init__(self) -> None:
        self.read: list[str] = []

    def _operations(self) -> Any:
        return _Named()

    async def _operate(self, method: Any, params: Any) -> Any:
        self.read.append(cast(str, method))
        if method == "ListDatasets":
            return ops.ListDatasetsResult(datasets=[ops.DatasetRef(dataset="dgm-1:daily")])
        if method == "ListPositions":
            return ops.ListPositionsResult(
                positions=[
                    ops.BookPosition(
                        account_id=account,
                        instrument_id="LCL-1",
                        trade_date_quantity=wire("10"),
                    )
                    for account in ("ACC-1", "ACC-9")
                ]
            )
        if method == "ListPrices":
            return ops.ListPricesResult(
                prices=[
                    ops.Price(
                        meta=meta("LCL-1"),
                        kind=ops.PRICE_KIND_CLOSE,
                        price=ops.Money(amount=wire("62431.27"), currency_code="USD"),
                    )
                ]
            )
        if method == "ListBars":
            usd = {"currency_code": "USD"}
            return ops.ListBarsResult(
                bars=[
                    ops.Bar(
                        meta=meta("LCL-1", 2),
                        open=ops.Money(amount=wire("62000"), **usd),
                        close=ops.Money(amount=wire("62431.27"), **usd),
                    ),
                    ops.Bar(
                        meta=meta("LCL-1", 1),
                        open=ops.Money(amount=wire("61000"), **usd),
                        close=ops.Money(amount=wire("62000"), **usd),
                    ),
                ]
            )
        raise AssertionError(f"the page read {method}, which no test here expects")


def client(sidecar: Sidecar | None = None) -> PageClient:
    """Ada, who may read ACC-1 through the plugin."""
    sidecar = sidecar or Sidecar()
    use(Report(cast(meridian.Plugin, sidecar)))
    return PageClient(pages, sidecar, read={"ACC-1"})


def test_each_page_is_served_at_its_levels_and_refused_at_the_others() -> None:
    served = {(r.page.path, r.level): r.response.status for r in client().every_page()}
    assert served == {
        ("/", "admin"): 403,
        ("/", "write"): 200,
        ("/", "read"): 200,
        ("/datasets", "admin"): 200,
        ("/datasets", "write"): 403,
        ("/datasets", "read"): 403,
    }


def test_closes_values_each_position_at_its_close_for_the_accounts_she_may_read() -> None:
    page = client().get("/", "read").text
    # Ten units at 62431.27: computed here and shown, never recorded.
    assert "<td><code>ACC-1</code></td><td><code>LCL-1</code></td><td>10</td>" in page
    assert "<td>62431.27</td><td>USD</td><td>624312.70</td>" in page
    # The week's change: the last bar's close less the first one's open.
    assert "<td>1431.27</td><td>dgm-1:daily</td>" in page
    # ACC-9 is in the plugin's scope, not hers.
    assert "ACC-9" not in page


def test_an_agent_reads_the_report_through_its_tool_at_each_level_of_the_page() -> None:
    # The same rows as the page, for an agent the person delegated to: under
    # Open and under View, and refused under Manage, as the page is.
    yesterday = (datetime.now(UTC).date() - timedelta(days=1)).isoformat()
    for level in ("write", "read"):
        answered = client().call_tool("read_report", level=level)
        assert answered.outcome == "unchanged", answered
        assert answered.data == {
            "business_date": yesterday,
            "rows": [
                {
                    "account": "ACC-1",
                    "instrument": "LCL-1",
                    "quantity": "10",
                    "close": "62431.27",
                    "currency": "USD",
                    "dataset": "dgm-1:daily",
                    "value": "624312.70",
                    "week": "1431.27",
                }
            ],
        }
    assert client().call_tool("read_report", level="admin").outcome == "refused"


def test_manage_shows_what_it_may_read_and_no_accounts_data() -> None:
    sidecar = Sidecar()
    page = client(sidecar).get("/datasets", "admin").text
    assert "dgm-1:daily" in page
    client(sidecar).assert_no_account_data("ACC-1", "62431.27")


def test_nothing_is_recorded_only_read() -> None:
    sidecar = Sidecar()
    client(sidecar).get("/", "write")
    client(sidecar).get("/datasets", "admin")
    assert set(sidecar.read) == {"ListPositions", "ListPrices", "ListBars", "ListDatasets"}
