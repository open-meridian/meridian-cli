"""The pages, against a stand-in for the sidecar: each served at its level,
each drawing the operations role's sections and "not set up yet" states; the
act that confirms an opening balance named with its check, so it can be
prepared, and the page opened from a prepared change starting from it."""

from __future__ import annotations

import json
from dataclasses import dataclass, field

import meridian
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.testing import PageClient
from meridian.v1 import sidecar_pb2

from reference_plugin.page import pages


@dataclass
class Sidecar:
    """Who the plugin was launched as: its breaks, what it recorded, and the
    prepared change the dashboard would show."""

    identity = meridian.Identity("reference-1", roles=("operations",))
    open: list[ops.Break] = field(default_factory=list)
    recorded: list[dict[str, object]] = field(default_factory=list)
    prepared: sidecar_pb2.PreparedCall | None = None

    async def list_breaks(self, **_: object) -> ops.ListBreaksResult:
        return ops.ListBreaksResult(breaks=self.open)

    async def record_opening_balance(self, **given: object) -> ops.RecordOpeningBalanceResult:
        self.recorded.append(given)
        return ops.RecordOpeningBalanceResult()

    async def _read_prepared(self, prepared_call_id: str, _: object) -> object:
        return self.prepared


def client(sidecar: Sidecar | None = None) -> PageClient:
    return PageClient(pages, sidecar or Sidecar(), read={"ACC-1"}, write={"ACC-1"})


def test_each_page_is_served_at_open_and_view_the_daily_page_first() -> None:
    assert [page.title for page in pages.declared] == [
        "Breaks",
        "Reconciliations",
        "Opening balances",
    ]
    served = {(r.page.path, r.level): r.response.status for r in client().every_page()}
    assert served[("/", "write")] == 200 and served[("/", "admin")] == 403


def test_breaks_say_the_next_step_until_there_are_some() -> None:
    assert "No breaks yet" in client().get("/", "read").text
    sidecar = Sidecar(
        open=[ops.Break(break_id="B-1", account_id="ACC-1", first_seen_date="2026-01-02")]
    )
    page = client(sidecar).get("/", "read").text
    assert "Open breaks" in page and "B-1" not in page and "2026-01-02" in page
    assert client(sidecar).call_tool("read_breaks").data == {
        "open_breaks": 1,
        "overdue_breaks": 1,
    }


def test_confirming_names_its_check_so_it_can_be_prepared() -> None:
    tool = client().tool("confirm_opening_balance").declared()
    assert tool.checked_by == "check_opening_draft"
    assert list(tool.accounts) == ["account_id"]
    assert tool.made_on == "/opening"
    checked = client().call_tool(
        "check_opening_draft", {"account_id": "ACC-1", "as_of_date": "2026-09-30"}
    )
    assert checked.data["may_confirm"] is True


def test_a_page_opened_from_a_prepared_change_starts_from_it() -> None:
    sidecar = Sidecar(
        prepared=sidecar_pb2.PreparedCall(
            prepared_call_id="PC-1",
            tool_name="confirm_opening_balance",
            arguments_json=json.dumps({"account_id": "ACC-1", "as_of_date": "2026-09-30"}),
            prepared_through_client="Claude",
            reason="The statement agrees.",
        )
    )
    page = client(sidecar).request("GET", "/opening", "write", prepared_call_id="PC-1").text
    assert "Prepared by Claude: The statement agrees." in page
    assert 'value="2026-09-30"' in page
    made = client(sidecar).call_tool(
        "confirm_opening_balance", {"account_id": "ACC-1", "as_of_date": "2026-09-30"}
    )
    assert made.outcome == "made" and sidecar.recorded[0]["account_id"] == "ACC-1"
