"""The `reporting` suite: the role's requirement, which this plugin holds
itself to.

Each case says, in the contract's words, what the deployment presents and
what the plugin reads or hears. Here each is run through the plugin's own
reads (report.py), against a `Recorder`: the plugin's typed operations,
recorded rather than sent, answered as `answer` says for the case. A row the
plugin hears is handed to its own `on_<row>` method, bound to the recorder.

As you change what the plugin reports, keep every case passing.
"""

from __future__ import annotations

from datetime import date
from typing import Any

from meridian.plugin.v1 import operations_pb2 as ops
from meridian.suites import Producer, Recorder, run

from reference_plugin.report import Report

DAY = date(2026, 10, 8)


def report(recorder: Recorder) -> Report:
    return recorder.bind(Report())


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
            price=ops.Price(meta=ops.ObservationMeta(source=ops.Source(dataset="data-1:daily")))
        ),
    )
    await recorder.hear(plugin)
    assert plugin.prices_heard == 1


async def a_bar_heard(recorder: Recorder) -> None:
    plugin = report(recorder)
    recorder.answer(
        "BarsRecorded",
        lambda _: ops.BarsRecordedEvent(
            bar=ops.Bar(meta=ops.ObservationMeta(source=ops.Source(dataset="data-1:daily")))
        ),
    )
    await recorder.hear(plugin)
    assert plugin.bars_heard == 1


async def reporting_currency(recorder: Recorder) -> None:
    # No position names the currency's cash instrument: it is resolved by its code.
    recorder.answer(
        "ResolveIdentifier",
        lambda _: ops.ResolveIdentifierResult(found=True, instrument_id="LCL-CASH-USD"),
    )
    assert await report(recorder).currency(DAY) == "LCL-CASH-USD"


async def a_job_run_and_read(recorder: Recorder) -> None:
    """Contract v26: the accounts' market value asked of the analytics
    component, answered later, its finishing heard and its answer read."""
    plugin = report(recorder)
    recorder.answer(
        "ListMeasureDefinitions",
        lambda _: ops.ListMeasureDefinitionsResult(
            definitions=[
                ops.MeasureDefinition(
                    definition="market_value", version=1, breakdowns=["asset_class"]
                )
            ]
        ),
    )
    recorder.answer("RunJob", lambda _: ops.RunJobResult(answered_inline=False, job_id="JOB-1"))
    recorder.answer("JobFinished", lambda _: ops.JobFinishedEvent(job_id="JOB-1", sequence=1))
    recorder.answer("ReadJobAnswer", lambda _: ops.ReadJobAnswerResult(job_id="JOB-1"))
    await recorder.hear(plugin)
    answer = await plugin.market_value(["ACC-1"])
    assert answer.job_id == "JOB-1"


async def finished_jobs_caught_up(recorder: Recorder) -> None:
    """Contract v26: the jobs finished after the last it saw, read after a loss."""
    plugin = report(recorder)
    plugin.jobs.sequence = 7
    await plugin.jobs.catch_up(recorder)


async def a_bounded_query(recorder: Recorder) -> None:
    """Contract v26: one bounded query of the lake, a kind a reporting plugin
    reads (prices, bars and outcomes only)."""
    await report(recorder).lake_rows("LCL-1")


async def a_curve_read(recorder: Recorder) -> None:
    """Contract v27: a rate curve's points for a business date."""
    await report(recorder).curve("INS-PAR", DAY)


async def a_rate_heard(recorder: Recorder) -> None:
    plugin = report(recorder)
    recorder.answer(
        "RatesRecorded",
        lambda _: ops.RatesRecordedEvent(
            rate=ops.Rate(meta=ops.ObservationMeta(source=ops.Source(dataset="rates-1:par")))
        ),
    )
    await recorder.hear(plugin)
    assert plugin.rates_heard == 1


async def a_liveness_heard(recorder: Recorder) -> None:
    """Contract v27: a stream's heartbeat, its quotes unchanged, confirms
    the prices in force."""
    plugin = report(recorder)
    recorder.answer(
        "LivenessRecorded",
        lambda _: ops.LivenessRecordedEvent(
            liveness=ops.DatasetLiveness(dataset="data-1:live", alive_at_ns=7)
        ),
    )
    await recorder.hear(plugin)
    assert plugin.liveness.at("data-1:live") == 7


async def costs_read(recorder: Recorder) -> None:
    """Contract v27: an account's costs against arrival for a trade date."""
    await report(recorder).costs("ACC-1", DAY)


# Contract v28: chains, lifecycle, derived series, saved scenarios and the
# filings slice, read and heard through the plugin's own reads.
AAPL, APPLE = "INS-AAPL", "ENT-APPLE"


def hearing(row: str, message: Any, arm: str, kind: str, counted: str) -> Producer:
    """A row heard on the plugin's own handler (`on_<kind>`), counted."""

    async def produce(recorder: Recorder) -> None:
        plugin = report(recorder)
        recorder.answer(row, lambda _: message)
        await recorder.hear(plugin)
        assert getattr(plugin, counted) == 1

    return produce


async def open_interest(recorder: Recorder) -> None:
    await report(recorder).open_interest(AAPL, DAY)


async def lifecycle_read(recorder: Recorder) -> None:
    await report(recorder).lifecycle("ACC-1", DAY, DAY)


async def derived_read(recorder: Recorder) -> None:
    await report(recorder).derived(AAPL, "iv30", DAY, DAY)


async def scenarios_read(recorder: Recorder) -> None:
    await report(recorder).scenarios("grid:price-by-date")


async def filings_read(recorder: Recorder) -> None:
    await report(recorder).filings(APPLE)


async def facts_read(recorder: Recorder) -> None:
    await report(recorder).facts(APPLE, "us-gaap:EarningsPerShareBasic")


async def holders_read(recorder: Recorder) -> None:
    await report(recorder).holders(AAPL, DAY)


async def dividends_read(recorder: Recorder) -> None:
    await report(recorder).dividends(AAPL, DAY)


async def ratings_read(recorder: Recorder) -> None:
    await report(recorder).ratings(APPLE)


POOL = "INS-01JC29UMBS0000000000000001"


async def factors_read(recorder: Recorder) -> None:
    await report(recorder).factors(POOL, DAY, DAY)


PRODUCERS: dict[str, Producer] = {
    "reads-the-datasets-it-may-read": datasets,
    "reads-a-business-dates-closes": valued,
    "hears-a-price-recorded": a_price_heard,
    "reads-daily-bars-over-a-range": valued,
    "hears-a-bar-recorded": a_bar_heard,
    "resolves-its-reporting-currency": reporting_currency,
    "runs-a-job-and-reads-its-answer": a_job_run_and_read,
    "catches-up-finished-jobs": finished_jobs_caught_up,
    "queries-the-lake-within-bounds": a_bounded_query,
    "reads-a-curve-for-a-date": a_curve_read,
    "hears-a-rate-recorded": a_rate_heard,
    "hears-a-datasets-liveness": a_liveness_heard,
    "reads-an-accounts-execution-costs": costs_read,
    "reads-open-interest-for-a-chain": open_interest,
    "hears-a-statistic-recorded": hearing(
        "StatisticsRecorded",
        ops.StatisticsRecordedEvent(statistic=ops.Statistic()),
        "statistics_recorded",
        "statistic",
        "statistics_heard",
    ),
    "reads-an-accounts-lifecycle-events": lifecycle_read,
    "hears-a-lifecycle-event-recorded": hearing(
        "LifecycleEventRecorded",
        ops.LifecycleEventRecordedEvent(event=ops.LifecycleEvent()),
        "lifecycle_event_recorded",
        "lifecycle_event",
        "lifecycle_heard",
    ),
    "reads-a-derived-series": derived_read,
    "reads-a-saved-scenario-set": scenarios_read,
    "reads-a-companys-filings": filings_read,
    "reads-reported-facts": facts_read,
    "reads-who-holds-a-security": holders_read,
    "reads-declared-dividends": dividends_read,
    "reads-a-rating-history": ratings_read,
    "hears-a-filing-recorded": hearing(
        "FilingsRecorded",
        ops.FilingsRecordedEvent(filing=ops.Filing()),
        "filings_recorded",
        "filing",
        "filings_heard",
    ),
    "hears-a-dividend-declared": hearing(
        "CorporateActionsRecorded",
        ops.CorporateActionsRecordedEvent(action=ops.CorporateActionAnnounced()),
        "corporate_actions_recorded",
        "dividend",
        "dividends_heard",
    ),
    # Contract v29: a pool's factors read, and each month's heard.
    "reads-a-pools-factors": factors_read,
    "hears-a-factor-recorded": hearing(
        "FactorsRecorded",
        ops.FactorsRecordedEvent(factor=ops.Factor()),
        "factors_recorded",
        "factor",
        "factors_heard",
    ),
}


#: A case about something this plugin never reports on is not presented,
#: with why, rather than mapped (its `about`): this board values prices, and
#: no event contract's outcome (contract v23); and from contract v29, when
#: every read carries its `about` (Track E), the reads its work never makes:
#: it values positions at a close, so it reads no orders, fills, order
#: histories, bookings or alerts, no settlement calendar and no settlement
#: date. From contract 30 it holds no standing figure (it values at a close,
#: not as inputs change) and shows no source priorities (it reads the
#: deployment's default). Report on any of them and map the case to the
#: plugin's own read.
_NO_ORDERS = "this board values positions at a close; it reads no orders"
NOT_PRESENTED: dict[str, str] = {
    "hears-and-reads-an-outcome": "this board values prices, and no event contract's outcome",
    "reads-orders-and-catches-up": _NO_ORDERS,
    "hears-an-order-and-an-execution": _NO_ORDERS,
    "reads-an-orders-history-for-the-person-roles": _NO_ORDERS,
    "hears-and-reads-bookings": "this board values positions at a close; it reads no bookings",
    "hears-and-reads-alerts": "this board values positions at a close; it reads no alerts",
    "reads-a-calendar": "this board values positions at a close; it reads no calendar",
    "asks-for-a-settlement-date": (
        "this board values positions at a close; it asks no settlement date"
    ),
    "holds-a-standing-figure": (
        "this board values positions at a close; it holds no figure kept current"
    ),
    "reads-the-source-priorities": (
        "this board reads the deployment's default source; it shows no priorities"
    ),
}


def test_every_case_of_the_reporting_suite_passes() -> None:
    report = run("reporting", PRODUCERS, not_presented=NOT_PRESENTED, instance_id="reference-1")
    assert report.passed, report.failures
