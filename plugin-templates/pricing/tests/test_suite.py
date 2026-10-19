"""The `pricing` suite: the role's requirement, which this plugin holds
itself to.

Each case says, in the contract's words, what the analytics component
hands the plugin and what the plugin answers. Here each is run through the
SDK's Pricer over this plugin's own models (model.py), against a
`Recorder`: the plugin's typed operations, recorded rather than sent, a
batch delivered to its own handler through `receive`.

As you change the plugin's models, keep every case passing.
"""

from __future__ import annotations

import asyncio
from decimal import Decimal
from typing import cast

from google.protobuf.message import Message

from meridian.operations import as_money
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.roles import pricing
from meridian.suites import Producer, Recorder, names, run

from reference_plugin.model import MODELS

AAPL = "INS-01J8XQ4M7K0000000000AAPL"
USD = "INS-01J8XQ4M7K00000000CASHUSD"


def wire(text: str) -> ops.Decimal:
    amount = Decimal(text)
    scale = max(0, -int(amount.as_tuple().exponent))
    return ops.Decimal(low=int(amount.scaleb(scale)), scale=scale)


def unit(
    close: str, kind: ops.ScenarioKind = ops.SCENARIO_KIND_BASE, day: str = ""
) -> ops.PricingUnit:
    return ops.PricingUnit(
        instrument_id=AAPL,
        template_version="2",
        scenario_key=ops.ScenarioKey(kind=kind, business_date=day),
        inputs=[
            ops.MarketObjectValue(
                subject_id=AAPL,
                kind="close",
                value=ops.Money(amount=wire(close), instrument_id=USD),
                dataset="data-1:daily",
            )
        ],
    )


def batch(*units: ops.PricingUnit, sequence: int = 17) -> ops.PriceBatchRequest:
    return ops.PriceBatchRequest(
        batch_id=f"PBT-{sequence}",
        pricing_instance_id="reference-1",
        model_id="quoted_price",
        model_version=1,
        units=list(units),
        sequence=sequence,
    )


def delivered(*batches: ops.PriceBatchRequest) -> Producer:
    async def produce(recorder: Recorder) -> None:
        recorder.answer("PriceBatch", lambda _: list(batches))
        await recorder.receive(price_batch=pricing.Pricer(recorder, MODELS).heard)

    return produce


async def after_a_restart(recorder: Recorder) -> None:
    kept = pricing.Sequence()
    kept.note(16)  # the last it answered before it stopped
    waiting = batch(unit("227.50"), sequence=17)

    def waiting_after(params: Message) -> Message:
        after = cast(ops.ListPriceBatchesParams, params).after_sequence
        return ops.ListPriceBatchesResult(batches=[waiting] if after < 17 else [], sequence=17)

    recorder.answer("ListPriceBatches", waiting_after)
    await pricing.Pricer(recorder, MODELS, sequence=kept).catch_up()


async def equals_the_built_in_model(recorder: Recorder) -> None:
    """The equity template's test case: 10 shares at a close of 227.50 USD,
    whose value per unit the built-in model answers 227.50 USD."""
    await delivered(batch(unit("227.50")))(recorder)
    (answered,) = cast(list[ops.AnswerPriceBatchParams], recorder.on("AnswerPriceBatch"))
    (value,) = answered.values
    assert as_money(value.value_per_unit).amount == Decimal("227.50")
    assert as_money(value.value_per_unit).instrument_id == USD


PRODUCERS: dict[str, Producer] = {
    "answers-a-batch": delivered(batch(unit("227.50"))),
    "answers-every-scenario-of-a-batch": delivered(
        batch(
            *(
                unit(close, ops.SCENARIO_KIND_HISTORICAL_DAY, day)
                for close, day in (("226.10", "2026-10-15"), ("229.05", "2026-10-16"))
            )
        )
    ),
    "equals-the-built-in-model": equals_the_built_in_model,
    "carries-no-sensitivity": delivered(batch(unit("227.50"))),
    "reads-the-batches-waiting-after-a-restart": after_a_restart,
}


def test_every_case_is_mapped() -> None:
    assert set(names("pricing")) == set(PRODUCERS)


def test_every_case_of_the_pricing_suite_passes() -> None:
    report = run("pricing", PRODUCERS, instance_id="reference-1")
    assert report.passed, report.failures


def test_a_model_id_it_does_not_implement_leaves_the_batch_unanswered() -> None:
    recorder = Recorder("reference-1")
    pricer = pricing.Pricer(recorder, MODELS)
    other = batch(unit("1.00"))
    other.model_id = "binary_event"
    assert asyncio.run(pricer.answer(other)) is None
    assert recorder.on("AnswerPriceBatch") == []
    assert "not implemented" in pricer.unanswered[other.batch_id]
