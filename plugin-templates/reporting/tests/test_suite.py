"""The `reporting` suite: the role's requirement, which this plugin holds
itself to.

Each case says, in the contract's words, what the deployment presents and
what the plugin reads or hears. Here each is run through the plugin's own
reads (report.py), against a `Recorder`: the plugin's typed operations,
recorded rather than sent, answered as `answer` says for the case. A row the
plugin hears is handed to its own handler, through `receive`.

As you change what the plugin reports, keep every case passing.
"""

from __future__ import annotations

from datetime import date
from typing import cast

import meridian
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.suites import Producer, Recorder, run

from reference_plugin.report import Report

DAY = date(2026, 10, 8)


def report(recorder: Recorder) -> Report:
    return Report(cast(meridian.Plugin, recorder))


async def datasets(recorder: Recorder) -> None:
    await report(recorder).datasets()


async def valued(recorder: Recorder) -> None:
    recorder.answer(
        "ListPositions",
        lambda _: ops.ListPositionsResult(
            positions=[ops.BookPosition(account_id="ACC-1", instrument_id="LCL-1")]
        ),
    )
    await report(recorder).rows(DAY)


async def a_price_heard(recorder: Recorder) -> None:
    plugin = report(recorder)
    recorder.answer(
        "PricesRecorded",
        lambda _: ops.PricesRecordedEvent(
            price=ops.Price(meta=ops.ObservationMeta(source=ops.Source(dataset="dgm-1:daily")))
        ),
    )
    await recorder.receive(prices_recorded=plugin.on_price, subjects=["LCL-1"])
    assert plugin.prices_heard == 1


async def a_bar_heard(recorder: Recorder) -> None:
    plugin = report(recorder)
    recorder.answer(
        "BarsRecorded",
        lambda _: ops.BarsRecordedEvent(
            bar=ops.Bar(meta=ops.ObservationMeta(source=ops.Source(dataset="dgm-1:daily")))
        ),
    )
    await recorder.receive(bars_recorded=plugin.on_bar, subjects=["LCL-1"])
    assert plugin.bars_heard == 1


async def reporting_currency(recorder: Recorder) -> None:
    # No position names the currency's cash instrument: it is resolved by its code.
    recorder.answer(
        "ResolveIdentifier",
        lambda _: ops.ResolveIdentifierResult(found=True, instrument_id="LCL-CASH-USD"),
    )
    assert await report(recorder).currency(DAY) == "LCL-CASH-USD"


PRODUCERS: dict[str, Producer] = {
    "reads-the-datasets-it-may-read": datasets,
    "reads-a-business-dates-closes": valued,
    "hears-a-price-recorded": a_price_heard,
    "reads-daily-bars-over-a-range": valued,
    "hears-a-bar-recorded": a_bar_heard,
    "resolves-its-reporting-currency": reporting_currency,
}


def test_every_case_of_the_reporting_suite_passes() -> None:
    report = run("reporting", PRODUCERS, instance_id="reference-1")
    assert report.passed, report.failures
