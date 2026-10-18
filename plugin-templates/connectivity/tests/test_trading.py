"""The plugin's work and its page against the suites' recorder, standing in
for the sidecar alone: the SDK's Trader over this plugin's own venue adapter
(`meridian.roles.connectivity.testing.Rig`). FAKES ONLY: the stand-in venue
in memory, never a real broker, never a token.

Replace these as you replace the venue: at least a placement sent once,
its fills into the book, and each page and tool."""

from __future__ import annotations

import asyncio
from typing import Any

from meridian.operations import as_decimal
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.roles.connectivity import Venue, testing
from meridian.suites import Recorder
from meridian.testing import PageClient

from reference_plugin.page import pages, running
from reference_plugin.venue import ACCOUNT, SOURCE, StandInVenue


class Rig(testing.Rig):
    instance = "reference-1"
    code = ACCOUNT
    source = SOURCE

    def connect(self, broker: Any) -> Venue:
        return broker or StandInVenue()


def test_a_placement_is_sent_once_and_its_fills_reach_the_book() -> None:
    recorder = Recorder("reference-1")
    rig = Rig(recorder)
    held = rig.placement(quantity="2")
    rig.holds(held)
    asyncio.run(rig.hear(held))
    asyncio.run(rig.poll(3))
    reported: list[Any] = recorder.on("ReportPlacement")
    events = [ops.PlacementEvent.Name(p.event) for p in reported]
    assert events[0] == "PLACEMENT_EVENT_ACKNOWLEDGED"
    fills: list[Any] = recorder.on("ReportExecution")
    assert [as_decimal(f.quantity) for f in fills] == [1, 1]
    assert [f.venue_execution_id for f in fills] == ["1:1", "1:2"]


def test_the_trading_page_and_its_tool() -> None:
    rig = Rig(Recorder("reference-1"))
    asyncio.run(rig.trader.start())
    running["trader"] = rig.trader
    client = PageClient(pages)
    assert client.get("/", "admin").status == 200 and client.get("/", "read").status == 403
    answered = client.call_tool("read_trading", level="admin").data
    assert [a["venue_account_code"] for a in answered["accounts"]] == [ACCOUNT]
    assert answered["accounts"][0]["trading"] is True
