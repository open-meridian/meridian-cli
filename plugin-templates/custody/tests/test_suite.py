"""The custody suite: the role's requirement, which this plugin holds itself
to.

A plugin holding `custody` is verified for it only by passing every case of
the suite the SDK carries, and `meridian plugin check --verified` fails one
with no test running it. Each case says, in the contract's words, what the
custodian presents and what the plugin sends. Here each is mapped to one of
the custodian's responses, run through the plugin's own conversion
(convert.py), against a `Recorder`: the plugin's typed operations, recorded
rather than sent, answered as a sidecar answers them in the ordinary case, or
as `answer` says for the case.

When you replace the stand-in custodian with your own, map each case to a
recorded or synthetic exchange with it, and keep every case passing. A case
asserting one value of a closed list your custodian never presents may be
named in `not_presented`, with why; this stand-in presents every one.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Any

from meridian.plugin.v1 import operations_pb2 as ops
from meridian.suites import Producer, Recorder, run

from reference_plugin.convert import ACTIVITY_KINDS, SYNC_STATES, Converter

INSTANCE = "reference-1"
DAY = 1_791_417_600_000_000_000  # 2026-10-09 00:00 UTC
CONNECTION = {"institution": "Reference custodian", "status": "OK", "accounts": ["REF-1001"]}


def account(**given: Any) -> dict[str, Any]:
    """An account as the custodian answers it, with `given` in place."""
    return {
        "id": "REF-1001",
        "name": "Individual brokerage",
        "type": "MARGIN",
        "type_name": "Margin",
        "institution": "Reference custodian",
        "base_currency": "USD",
        "as_of": "2026-10-09",
        "positions": [],
        **given,
    }


def position(**given: Any) -> dict[str, Any]:
    return {
        "symbol": "QQQ",
        "kind": "ETF",
        "units": "10",
        "settled": "10",
        "currency": "USD",
        **given,
    }


def converted(step: Callable[[Converter], Awaitable[None]]) -> Producer:
    async def produce(recorder: Recorder) -> None:
        await step(Converter(recorder))

    return produce


def synced(status: str) -> Producer:
    return converted(lambda c: c.sync({**CONNECTION, "status": status}))


def kind(code: str, name: str) -> Producer:
    return converted(lambda c: c.accounts([account(type=code, type_name=name)]))


def holdings(**given: Any) -> Producer:
    return converted(lambda c: c.statement(account(**given), CONNECTION, DAY))


def activity(code: str, **given: Any) -> Producer:
    item = {"id": f"ACT-{code}", "type": code, "date": "2026-10-08", "currency": "USD", **given}
    return converted(lambda c: c.activity("REF-1001", item, []))


async def ambiguous(recorder: Recorder) -> None:
    recorder.answer(
        "ResolveIdentifier",
        lambda _: ops.ResolveIdentifierResult(
            found=False, miss_reason=ops.MISS_REASON_AMBIGUOUS
        ),
    )
    await Converter(recorder).statement(
        account(positions=[position(kind="WARRANT", kind_name="Warrant")]), CONNECTION, DAY
    )


async def read_for_linking(recorder: Recorder) -> None:
    await recorder.read_accounts_for_linking()


PLAN = {
    "id": "ACT-PLAN",
    "type": "BUY",
    "date": "2026-10-08",
    "units": "3",
    "plan_code": "SVF-1",
    "plan_name": "Stable value fund",
}
LINK = {"code": "SVF-1", "instrument": "LCL-SVF", "changed_by": "Ada Park"}

CODES = {"sync-" + state.replace("_", "-"): code for code, state in SYNC_STATES.items()}
ACTIVITIES = {"activity-" + k.replace("_", "-"): code for code, k in ACTIVITY_KINDS.items()}

PRODUCERS: dict[str, Producer] = {
    **{name: synced(code) for name, code in CODES.items()},
    "account-kind-cash": kind("CASH", "Cash"),
    "account-kind-margin": kind("MARGIN", "Margin"),
    "account-kind-retirement": kind("ROTH", "Roth IRA"),
    "account-kind-not-known": kind("TRUST", "Trust"),
    "read-accounts-for-linking": read_for_linking,
    "link-a-new-account": converted(lambda c: c.link_new(account(), "Main", None)),
    "a-security-holding": holdings(positions=[position()]),
    "institution-not-stated": holdings(institution="", positions=[position()]),
    "cash-net-of-a-fund-counted-in-cash": holdings(
        positions=[
            position(
                symbol="REFXX", kind="MMF", units="500", settled="500", counted_in_cash=True
            )
        ],
        cash={"amount": "1523.00", "currency": "USD"},
    ),
    "fund-larger-than-cash": holdings(
        positions=[position(symbol="REFXX", kind="MMF", units="2000", counted_in_cash=True)],
        cash={"amount": "1523.00", "currency": "USD"},
    ),
    "currency-not-stated": holdings(positions=[position(currency="", average_cost="412.50")]),
    "settled-quantity-not-stated": holdings(positions=[position(settled=None)]),
    "pending-by-value-date": holdings(
        positions=[position(settled=None, pending=[{"value_date": "2026-10-10", "units": "4"}])]
    ),
    "resolve-states-what-the-source-states": holdings(positions=[position()]),
    "kind-not-converted-on-an-ambiguous-resolve": ambiguous,
    **{name: activity(code) for name, code in ACTIVITIES.items()},
    "activity-kind-not-converted": activity("ODD", type_name="Odd lot adjustment"),
    "activity-signs": activity("SELL", units="-5", price="412.10", amount="2060.50"),
    "activity-moves-no-cash": activity("SPLIT", units="10"),
    "activity-plan-code-not-linked": converted(lambda c: c.activity("REF-1001", PLAN, [])),
    "activity-plan-code-linked-by-a-person": converted(
        lambda c: c.activity("REF-1001", PLAN, [LINK])
    ),
    "activity-re-resolved-when-a-plan-code-is-linked-later": converted(
        lambda c: c.plan_code_linked("REF-1001", PLAN, LINK, DAY)
    ),
    "history-from-stated": converted(
        lambda c: c.sync({**CONNECTION, "history_from": "2019-01-02"})
    ),
}


def test_every_case_of_the_custody_suite_passes() -> None:
    report = run("custody", PRODUCERS, instance_id=INSTANCE)
    assert report.passed, report.failures
    assert len(report.passed_cases) == 42
