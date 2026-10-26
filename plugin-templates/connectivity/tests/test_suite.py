"""The `connectivity` suite: the role's requirement, which this plugin holds
itself to, so `meridian plugin check --verified` passes on it as written.

Each case says, in the contract's words, what the venue presents and what
the plugin sends. Here each is run through the SDK's Trader over this
plugin's own venue adapter (venue.py), against the runner's recorder,
which stands in for the sidecar alone; what the stand-in venue answers for
a case is chosen by the symbol the placement's instrument is named by
(`Answering`). FAKES ONLY: nothing reaches a broker.

A case marked `about` or `about_field` that this venue never presents is
named in `NOT_PRESENTED`, with why. As you replace the stand-in with your
venue, map each case to what your venue answers for it, and keep every
case passing.
"""

from __future__ import annotations

import asyncio
from dataclasses import replace as changed
from typing import Any

from meridian.plugin.v1 import operations_pb2 as ops
from meridian.roles.connectivity import (
    Asked,
    Venue,
    VenueOrder,
    VenueRefused,
    VenueUnanswered,
    testing,
)
from meridian.suites import ANSWERED_BY_SDK, Producer, Recorder, names, run, suite

from reference_plugin.venue import ACCOUNT, SOURCE, StandInVenue

#: The symbols the stand-in answers a case for.
REJECTED, ODD, LOST, RENAMED = "REJECTED", "ODD", "LOST", "RENAMED"


class Answering(StandInVenue):
    """The stand-in venue, answering each case's symbol as the case says:
    a refusal with the venue's own reason, a status the mapping does not
    know, a send never answered that reached the venue, and a fill under a
    symbol renamed while the order worked."""

    def place(self, code: str, asked: Asked) -> tuple[VenueOrder, str]:
        if asked.symbol == REJECTED:
            raise VenueRefused("insufficient_buying_power", "not enough buying power",
                               self.keep("refused", {"symbol": asked.symbol}))  # fmt: skip
        order, raw = super().place(code, asked)
        if asked.symbol == ODD:
            order = changed(order, status="held_for_review")
            self.held[order.order_id] = order
        if asked.symbol == LOST:
            raise VenueUnanswered("the send timed out", raw)
        return order, raw

    def orders(self, code: str) -> tuple[list[VenueOrder], str]:
        held, raw = super().orders(code)
        return [
            changed(each, symbol=f"{RENAMED}.B") if each.symbol == RENAMED else each
            for each in held
        ], raw


class Rig(testing.Rig):
    instance = "reference-1"
    code = ACCOUNT
    source = SOURCE

    def connect(self, broker: Any) -> Venue:
        return broker or Answering()


NOT_PRESENTED = {
    "reports-a-bust": "the stand-in venue never takes a fill back",
    "reports-a-charge": "the stand-in venue charges nothing",
    "reads-whether-releases-wait-for-compliance": (
        "it places what its ems released; whether releases wait for compliance changes "
        "nothing it sends"
    ),
    "hears-an-accounts-attributes-change": (
        "it places what its ems released; whether releases wait for compliance changes "
        "nothing it sends"
    ),
    "records-a-locate": "the stand-in venue offers no locate",
    "resolves-a-venue-symbol-by-the-facts-date": "the stand-in venue offers no locate",
    "resolves-an-executions-venue": "the stand-in venue states no fill's place of execution",
    "reports-a-venue-not-held": "the stand-in venue states no fill's place of execution",
    "reads-a-calendar": (
        "its fills state no settlement date, so it asks the date read for each (ComputeDate); "
        "it reads no calendar's days"
    ),
    "reports-a-fill-on-no-placement": "the stand-in venue holds only the orders it was sent",
    "reports-a-later-fill-on-the-accepted-placement": (
        "the stand-in venue holds only the orders it was sent"
    ),
    "re-reports-a-charge-on-the-accepted-placement": "the stand-in venue charges nothing",
    "reports-a-value-date-roll": "the stand-in venue rolls no spot position",
    "reports-a-give-up-rejected": "the stand-in venue clears every order it fills",
    "records-a-dealers-quote-on-an-rfq-placement": "the stand-in venue takes no RFQ",
    # Contract v28 (ruling 602): about options and packages.
    "reports-an-expiry": "the stand-in venue trades no options",
    "reports-a-package-fill": "the stand-in venue takes no package order",
}


def placed(symbol: str = "AAPL", quantity: str = "10", polls: int = 0, **kw: Any) -> Producer:
    """A placement to this instance, heard, sent and read `polls` times."""

    async def produce(recorder: Recorder) -> None:
        rig = Rig(recorder, symbol)
        held = rig.placement(quantity=quantity, **kw)
        rig.holds(held)
        await rig.hear(held)
        await rig.poll(polls)

    return produce


async def another_instances(recorder: Recorder) -> None:
    rig = Rig(recorder)
    held = rig.placement(instance="elsewhere")
    rig.holds(held)
    await rig.hear(held)
    await rig.poll()


async def the_same_fill_twice(recorder: Recorder) -> None:
    """One fill read, then read again by the plugin started again before the
    book held it: the same derived ID both times."""
    rig = Rig(recorder)
    held = rig.placement(quantity="1")
    rig.holds(held)
    await rig.hear(held)
    await rig.poll()
    worked = next(iter(rig.trader.working.values()))
    again = rig.placement(
        quantity="1", state=ops.PLACEMENT_STATE_WORKING, venue_order_id=worked.venue_order_id
    )
    rig.holds(again)
    rig.executions.clear()
    rig.trader = rig.make()
    await rig.trader.start()
    await rig.poll()


async def renamed(recorder: Recorder) -> None:
    """Filled under a symbol renamed while it worked, which names none of the
    deployment's records."""
    recorder.answer(
        "ResolveIdentifier",
        lambda _: ops.ResolveIdentifierResult(
            found=False, miss_reason=ops.MISS_REASON_NOT_FOUND
        ),
    )
    await placed(RENAMED, "1", 1)(recorder)


async def paused(recorder: Recorder) -> None:
    rig = Rig(recorder)
    await rig.instruct(ops.TRADING_INSTRUCTION_VALUE_PAUSE)
    held = rig.placement()
    rig.holds(held)
    await rig.hear(held)


async def stopped(recorder: Recorder) -> None:
    rig = Rig(recorder)
    held = rig.placement(quantity="200")
    rig.holds(held)
    await rig.hear(held)
    await rig.instruct(ops.TRADING_INSTRUCTION_VALUE_STOP)


async def started(recorder: Recorder) -> None:
    await Rig(recorder).trader.start()


async def caught_up(recorder: Recorder) -> None:
    rig = Rig(recorder)
    await rig.trader.start()
    await rig.poll()


async def round_trip(recorder: Recorder) -> None:
    """The session's latency over an interval -- each venue call's round
    trip, a placement's acknowledgement -- as the SDK's Trader recorded it,
    sent by `Latency.report` (required from contract v28, ruling 518)."""
    rig = Rig(recorder)
    held = rig.placement()
    rig.holds(held)
    await rig.hear(held)
    await rig.poll()
    assert await rig.latency.report() > 0


PRODUCERS: dict[str, Producer] = {
    "acknowledges-a-placement-naming-it": placed(),
    "hears-no-placement-of-another-instance": another_instances,
    "sends-the-venue-account-code-its-ems-set": placed(),
    "rejects-what-the-venue-does-not-offer": placed(tif=ops.TIME_IN_FORCE_GOOD_TILL_DATE),
    "rejects-a-strategy-its-broker-lacks": placed(
        strategy=ops.StrategyInstruction(kind=ops.STRATEGY_KIND_VWAP)
    ),
    "venue-rejected-with-its-reason-as-reported": placed(REJECTED),
    "unmapped-status-is-outcome-unknown": placed(ODD),
    "resolves-an-unanswered-send-by-asking": placed(LOST, "200", 1),
    "reports-a-fill-with-its-settlement-date": placed(polls=2),
    "derives-the-same-execution-id-twice": the_same_fill_twice,
    "sends-nothing-new-on-a-pause": paused,
    "cancels-at-the-venue-on-a-stop": stopped,
    "publishes-its-catalogue": started,
    "reads-the-instrument-it-places": placed(),
    "reads-its-placements-orders-on-start": started,
    "catches-up-placements-after-a-loss": caught_up,
    "reads-the-instructions-in-force-on-start": started,
    "reports-a-symbol-it-cannot-resolve": renamed,
    "reports-a-sessions-round-trip": round_trip,
}


def test_every_case_is_mapped_or_said_not_presented() -> None:
    assert set(names("connectivity")) == set(PRODUCERS) | set(NOT_PRESENTED) | set(
        ANSWERED_BY_SDK
    )
    held = {case.name: case.about or case.about_field for case in suite("connectivity").cases}
    assert all(held[name] for name in NOT_PRESENTED), "a required case is never skipped"


def test_every_case_of_the_connectivity_suite_passes() -> None:
    report = run(
        "connectivity", PRODUCERS, not_presented=NOT_PRESENTED, instance_id="reference-1"
    )
    assert report.passed, report.failures
    assert set(report.passed_cases) == set(PRODUCERS) | set(ANSWERED_BY_SDK)


def test_a_send_never_answered_is_asked_about_never_sent_twice() -> None:
    recorder = Recorder("reference-1")
    venue = Answering()
    rig = Rig(recorder, LOST, broker=venue)
    held = rig.placement(quantity="200")
    rig.holds(held)
    asyncio.run(rig.hear(held))
    asyncio.run(rig.poll())
    reported: list[Any] = recorder.on("ReportPlacement")
    assert ops.PlacementEvent.Name(reported[-1].event) == "PLACEMENT_EVENT_ACKNOWLEDGED"
    assert len(venue.held) == 1, "the venue received one order, never a second"
