"""The `dgm` suite: the role's requirement, which this plugin holds itself to.

A plugin holding `dgm` is verified for it only by passing every case of the
suite the SDK carries, and `meridian plugin check --verified` fails one with
no test running it. Each case says, in the contract's words, what the
vendor presents and what the plugin sends. Here each is mapped to one of the
vendor's responses (vendor.py), run through the plugin's own conversion
(convert.py), against a `Recorder`: the plugin's typed operations, recorded
rather than sent, answered as a sidecar answers them in the ordinary case,
or as `answer` says for the case.

When you replace the stand-in vendor with your own, map each case to a
recorded or synthetic exchange with it, and keep every case passing.
"""

from __future__ import annotations

from typing import cast

import meridian
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.suites import Producer, Recorder, run

from reference_plugin.convert import Converter
from reference_plugin.vendor import DAILY, FORMING

INSTANCE = "reference-1"
DAY = 1_791_417_600_000_000_000  # 2026-10-09 00:00 UTC, when the lake asks


def converter(recorder: Recorder) -> Converter:
    return Converter(cast(meridian.Plugin, recorder))


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
        await recorder.receive(observations_wanted=plugin.on_want)

    return produce


async def withdrawn(recorder: Recorder) -> None:
    plugin = converter(recorder)
    recorder.answer(
        "WantWithdrawn",
        lambda _: [ops.WantWithdrawnEvent(want_id="WNT-1", dataset=f"{INSTANCE}:daily")],
    )
    await recorder.receive(want_withdrawn=plugin.on_withdrawn)


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
    "a-subject-declined": wanted(None),
    "a-standing-want-withdrawn": withdrawn,
}


def test_every_case_of_the_dgm_suite_passes() -> None:
    report = run("dgm", PRODUCERS, instance_id=INSTANCE)
    assert report.passed, report.failures
