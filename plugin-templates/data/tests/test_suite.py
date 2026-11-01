"""The data suite: the role's requirement, which this plugin holds itself to.

A plugin holding `data` is verified for it only by passing every case of the
suite the SDK carries, and `meridian plugin check --verified` fails one with
no test running it. Each case says, in the contract's words, what the
vendor presents and what the plugin sends. Here each is mapped to one of the
vendor's responses (vendor.py), run through the plugin's own conversion
(convert.py), against a `Recorder`: the plugin's typed operations, recorded
rather than sent, answered as a sidecar answers them in the ordinary case,
or as `answer` says for the case.

When you replace the stand-in vendor with your own, map each case to a
recorded or synthetic exchange with it, and keep every case passing.

A case about a kind of data (its `about`) the plugin declares it never
publishes is not presented, with why, rather than mapped: this plugin's one
dataset is daily closes and bars (declaration.py), so it states no trades,
no quotes, no event contract's outcomes and no rates, and it is pulled,
never streamed. Declare a dataset of trades, quotes, outcomes or rates, or
stream one, and those cases are required again, and fail until each is
mapped. From contract v27 the lake's bounded query is about trades too: a
plugin recording none marks it not presented. And a case about a kind of
work it never does (contract v22's calendars and settlement dates; from
contract v29 every read carries its `about`) is not presented, with why,
rather than mapped to a read the plugin never makes.
"""

from __future__ import annotations

from meridian.plugin.v1 import operations_pb2 as ops
from meridian.roles import Latency
from meridian.suites import Producer, Recorder, run, suite

from reference_plugin.convert import Converter
from reference_plugin.declaration import DECLARATION
from reference_plugin.vendor import DAILY, FORMING

INSTANCE = "reference-1"
DAY = 1_791_417_600_000_000_000  # 2026-10-09 00:00 UTC, when the lake asks


def converter(recorder: Recorder) -> Converter:
    """The plugin's data role class, bound to the recorder (SDK 0.35)."""
    return recorder.bind(Converter())


def daily(symbol: str) -> Producer:
    """The vendor's candle for `symbol`, converted and recorded."""

    async def produce(recorder: Recorder) -> None:
        await converter(recorder).record_daily(symbol, DAILY[symbol])

    return produce


async def forming(recorder: Recorder) -> None:
    for text in FORMING:
        await converter(recorder).record_daily("BTC-USD", text)


async def ambiguous(recorder: Recorder) -> None:
    # The vendor's symbol meets two of the deployment's records.
    recorder.answer(
        "ResolveIdentifier",
        lambda _: ops.ResolveIdentifierResult(
            found=False, miss_reason=ops.MISS_REASON_AMBIGUOUS
        ),
    )
    await converter(recorder).record_daily("BTC-USD", DAILY["BTC-USD"])


async def venue_not_held(recorder: Recorder) -> None:
    recorder.answer(
        "ResolveVenue",
        lambda _: ops.ResolveVenueResult(found=False, miss_reason=ops.MISS_REASON_NOT_FOUND),
    )
    await converter(recorder).record_daily("QQQ", DAILY["QQQ"])


def wanted(symbol: str | None) -> Producer:
    """The lake wanting a business date's closes for the instrument `symbol`
    resolved to; for one the vendor does not serve, with None."""

    async def produce(recorder: Recorder) -> None:
        plugin = converter(recorder)
        await plugin.map_products(DAY)
        subject = next(
            (held for held, served in plugin.products.items() if served == symbol),
            "LCL-NOT-SERVED",
        )
        want = ops.ObservationsWantedEvent(
            want_id="WNT-1",
            dataset=f"{INSTANCE}:daily",
            data_type="meridian.v1.Price",
            subjects=[ops.SubjectRef(entity_id=subject)],
            kinds=[ops.PRICE_KIND_CLOSE],
            business_date="2026-10-08",
        )
        recorder.answer("ObservationsWanted", lambda _: want)
        await recorder.hear(plugin)

    return produce


async def elsewhere(recorder: Recorder) -> None:
    """The lake wanting a close for an instrument another source resolved --
    a custodian's QQQ, known by its symbol and a CUSIP -- which this plugin
    never resolved: it reads the record and serves its own symbol."""
    plugin = converter(recorder)
    subject = "LCL-HELD-ELSEWHERE"
    recorder.answer(
        "ResolveInstrument",
        lambda _: ops.ResolveInstrumentResult(
            found=True,
            instrument=ops.InstrumentRecord(
                instrument_id=subject,
                identifiers=[
                    ops.Identifier(scheme="cusip", value="46090E103"),
                    ops.Identifier(scheme="symbol", value="QQQ", source="custodian"),
                ],
            ),
        ),
    )
    want = ops.ObservationsWantedEvent(
        want_id="WNT-1",
        dataset=f"{INSTANCE}:daily",
        data_type="meridian.v1.Price",
        subjects=[ops.SubjectRef(entity_id=subject)],
        kinds=[ops.PRICE_KIND_CLOSE],
        business_date="2026-10-08",
    )
    recorder.answer("ObservationsWanted", lambda _: want)
    await recorder.hear(plugin)


async def withdrawn(recorder: Recorder) -> None:
    plugin = converter(recorder)
    recorder.answer(
        "WantWithdrawn",
        lambda _: [ops.WantWithdrawnEvent(want_id="WNT-1", dataset=f"{INSTANCE}:daily")],
    )
    await recorder.hear(plugin)


async def licences(recorder: Recorder) -> None:
    # On start (contract v20): its own declared datasets' licences.
    await converter(recorder).read_licences()


# Its feed delay (required from contract v28, ruling 518): each row it
# records unasked has its age at the edge -- when it was read, less the
# vendor's published time -- recorded by the SDK's Latency on its Wants, and
# the interval's snapshots sent (`Latency.report`).
async def its_feed_delay(recorder: Recorder) -> None:
    plugin = converter(recorder)
    plugin.wants.latency = Latency(recorder)
    await plugin.record_daily("BTC-USD", DAILY["BTC-USD"])
    assert await plugin.wants.latency.report() > 0


PRODUCERS: dict[str, Producer] = {
    "a-daily-close": daily("BTC-USD"),
    "the-forming-day-restated": forming,
    "an-exact-price": daily("SHIB-USD"),
    "a-daily-bar": daily("BTC-USD"),
    "an-fx-rate": daily("EUR-USD"),
    "a-stablecoin-quote": daily("ETH-USDC"),
    "an-asset-that-does-not-resolve": ambiguous,
    "a-venue-resolved": daily("QQQ"),
    "a-venue-not-held": venue_not_held,
    "a-want-recorded-against": wanted("BTC-USD"),
    "a-subject-another-source-resolved": elsewhere,
    "a-subject-declined": wanted(None),
    "a-standing-want-withdrawn": withdrawn,
    "reads-its-datasets-licences": licences,
    "reports-its-feed-delay": its_feed_delay,
}


#: Each kind of data a case may be about that is one of the lake's data
#: types, by its message: a case about one the catalogue does not declare is
#: about something this plugin never publishes.
KINDS = {
    "trades": "meridian.v1.Trade",
    "quotes": "meridian.v1.Quote",
    "outcomes": "meridian.v1.Outcome",
    "disputed outcomes": "meridian.v1.Outcome",
    "rates": "meridian.v1.Rate",
    # Contract v28 (ruling 602).
    "option chains": "meridian.v1.ListedSeries",
    "filings": "meridian.v1.Filing",
    "holdings": "meridian.v1.HoldingsReport",
    "corporate actions": "meridian.v1.CorporateActionAnnounced",
    "ratings": "meridian.v1.RatingAction",
    # Contract 28, amended 2026-10-31 (ruling 955): a publisher's figure.
    "derived series": "meridian.v1.DerivedSeries",
    # Contract v29: an agency's factor for a pool or a REMIC class.
    "factors": "meridian.v1.Factor",
}
DECLARED = {kind for dataset in DECLARATION.catalogue for kind in dataset.data_types}
STREAMED = any(
    mode == ops.OBSERVATION_MODE_STREAM
    for dataset in DECLARATION.catalogue
    for mode in dataset.modes
)

NOT_PRESENTED: dict[str, str] = {
    case.name: f"its datasets publish daily closes and bars, and no {case.about}"
    for case in suite("data").cases
    if case.about in KINDS and KINDS[case.about] not in DECLARED
}
if not STREAMED:
    NOT_PRESENTED["records-a-datasets-liveness"] = (
        "its datasets are pulled when wanted, never streamed"
    )
# What its work never does (contract v22's `about`, honestly marked from
# contract v29, Track E): it records what its vendor states, and reads no
# settlement calendar and asks no settlement date. A plugin that does maps
# each case to its own read.
NOT_PRESENTED["reads-a-calendar"] = "it records what its vendor states; it reads no calendar"
NOT_PRESENTED["asks-for-a-settlement-date"] = (
    "it records what its vendor states; it asks no settlement date"
)


def test_every_case_of_the_data_suite_passes() -> None:
    report = run("data", PRODUCERS, not_presented=NOT_PRESENTED, instance_id=INSTANCE)
    assert report.passed, report.failures
