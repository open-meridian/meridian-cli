"""What this plugin reports: the book's positions at a day's close, each with
the close the lake holds for it, and the change over the week to that day.

A `reporting` plugin reads the book and the lake and relates their records;
it never records anything back, and never matches orders. What it computes
-- here a position's value at the close -- is computed and shown, never
recorded. It reads prices by the deployment's source priority, which a
deployment admin sets on the dashboard's Data sources page, from the
datasets it is entitled to read.

Every number stays a `Decimal`, as the lake keeps it: never a `float`.

It names its reporting currency by its ISO 4217 code and has core resolve
that to the deployment's cash instrument (`resolve_identifier`, read-only
for a `reporting` plugin from contract v19), rather than finding it among
what the book happens to hold.

From contract v26 it asks the analytics component for what it values
(`market_value`), a job answered inline or later, each figure exact and
named with what it was computed from (`meridian.analytics`); and it asks the
lake one bounded query of the closes it shows (`lake_rows`).

From contract v27 it reads a rate curve's points for a date (`curve`, the
SDK's `rates.read_curve`), hears each rate the lake records and each
dataset's liveness (a stream's heartbeat, which confirms the prices in
force; `roles.Liveness`), and reads an account's execution costs against
the standard benchmarks for a trade date (`costs`), which the analytics
component records in the lake.

`meridian.suites.run("reporting", producers)` holds this to the role's
suite (tests/test_suite.py), against a recorder standing in for the sidecar.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, date, datetime, timedelta
from decimal import Decimal

import meridian
from meridian import analytics
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.roles import Liveness, reporting
from meridian.roles.data import rates

NS = 1_000_000_000


def _start(day: date) -> int:
    """The moment `day` begins, in UTC, in nanoseconds."""
    return int(datetime(day.year, day.month, day.day, tzinfo=UTC).timestamp()) * NS


def _in(money: ops.Money) -> str:
    """What an amount is in: its ISO 4217 code, or its cash instrument."""
    return money.currency_code or money.instrument_id


@dataclass(frozen=True)
class Row:
    """One position at the close: its account and instrument, its quantity,
    the close and the dataset it came from, the value it makes, and the
    change over the week; a figure the lake did not answer is None."""

    account: str
    instrument: str
    quantity: Decimal
    close: Decimal | None = None
    currency: str = ""
    dataset: str = ""
    value: Decimal | None = None
    week: Decimal | None = None


@dataclass
class Report:
    """The plugin's reads, against whatever operations it is given: the
    plugin's own when it runs, a recorder's in its tests."""

    plugin: meridian.Plugin
    #: The currency it reports in, by its ISO 4217 code.
    reporting_currency: str = "USD"
    #: What the lake has recorded since the plugin started, as heard.
    prices_heard: int = 0
    bars_heard: int = 0
    last_heard: str = ""
    datasets_read: list[str] = field(default_factory=list)
    #: Its analytics jobs, answered inline or later (contract v26).
    jobs: analytics.Jobs = field(default_factory=analytics.Jobs)
    #: Contract v27: the rates heard, and each dataset's latest liveness.
    rates_heard: int = 0
    liveness: Liveness = field(default_factory=Liveness)

    async def datasets(self) -> list[str]:
        """The datasets this plugin may read, by name."""
        return [each.dataset for each in await self.entitled()]

    async def entitled(self) -> list[reporting.DatasetRead]:
        """Each dataset this plugin may read, whether it is entitled to it
        and to which fields (none: every field): the reporting role's
        Datasets page's rows."""
        read = await self.plugin.list_datasets()
        self.datasets_read = [ref.dataset for ref in read.datasets]
        mine = {each.dataset: each for each in read.entitlements}
        return [
            reporting.DatasetRead(
                dataset=name,
                entitled=name in mine and mine[name].allowed,
                fields=list(mine[name].fields) if name in mine else [],
            )
            for name in self.datasets_read
        ]

    async def currency(self, day: date) -> str:
        """The deployment's cash instrument for the reporting currency on
        `day`, resolved by its ISO 4217 code whether or not any position
        holds it; empty where the deployment holds none."""
        found = await self.plugin.resolve_identifier(
            identifiers=[meridian.Identifier(scheme="iso4217", value=self.reporting_currency)],
            as_of_ns=_start(day),
        )
        return str(found.instrument_id) if found.found else ""

    async def positions(self, day: date) -> list[ops.BookPosition]:
        """Every position in the plugin's account scope on `day`, page by page."""
        held: list[ops.BookPosition] = []
        cursor = ""
        while True:
            page = await self.plugin.list_positions(business_date=day, cursor=cursor)
            held += [position for position in page.positions if not position.removed]
            cursor = page.next_cursor
            if not cursor:
                return held

    async def closes(self, subjects: list[str], day: date) -> dict[str, ops.Price]:
        """Each subject's close for `day`, by the deployment's source priority."""
        found: dict[str, ops.Price] = {}
        cursor = ""
        while True:
            page = await self.plugin.list_prices(
                subjects=[meridian.SubjectRef(entity_id=s) for s in subjects],
                kinds=["close"],
                business_date=day,
                cursor=cursor,
            )
            for price in page.prices:
                for ref in price.meta.subjects:
                    found[ref.entity_id] = price
            cursor = page.next_cursor
            if not cursor:
                return found

    async def weeks(self, subjects: list[str], day: date) -> dict[str, Decimal]:
        """Each subject's change over the seven days to `day`: the last
        daily bar's close less the first one's open."""
        bars: dict[str, list[ops.Bar]] = {}
        cursor = ""
        while True:
            page = await self.plugin.list_bars(
                subjects=[meridian.SubjectRef(entity_id=s) for s in subjects],
                interval_ns=86_400 * NS,
                valid_from_ns=_start(day - timedelta(days=6)),
                valid_until_ns=_start(day + timedelta(days=1)),
                cursor=cursor,
            )
            for bar in page.bars:
                for ref in bar.meta.subjects:
                    bars.setdefault(ref.entity_id, []).append(bar)
            cursor = page.next_cursor
            if not cursor:
                break
        changes: dict[str, Decimal] = {}
        for subject, held in bars.items():
            held.sort(key=lambda bar: bar.meta.valid_from_ns)
            first, last = held[0].open, held[-1].close
            if _in(first) == _in(last):
                changes[subject] = meridian.as_decimal(last.amount) - meridian.as_decimal(
                    first.amount
                )
        return changes

    async def rows(self, day: date) -> list[Row]:
        """Every position on `day`, valued at its close where the lake holds
        one: computed here, shown, and never recorded."""
        positions = await self.positions(day)
        subjects = sorted({position.instrument_id for position in positions})
        if not subjects:
            return []
        closes = await self.closes(subjects, day)
        weeks = await self.weeks(subjects, day)
        rows = []
        for position in positions:
            quantity = meridian.as_decimal(position.trade_date_quantity)
            price = closes.get(position.instrument_id)
            if price is None:
                rows.append(Row(position.account_id, position.instrument_id, quantity))
                continue
            close = meridian.as_decimal(price.price.amount)
            rows.append(
                Row(
                    account=position.account_id,
                    instrument=position.instrument_id,
                    quantity=quantity,
                    close=close,
                    currency=_in(price.price),
                    dataset=price.meta.source.dataset,
                    value=quantity * close,
                    week=weeks.get(position.instrument_id),
                )
            )
        return rows

    # ── The analytics component and the lake's query (contract v26) ─────

    async def market_value(self, accounts: list[str]) -> analytics.Answer:
        """Each account's market value by asset class, as the analytics
        component computes it: exact, never recomputed here."""
        known = analytics.measures_of(
            (await self.plugin.list_measure_definitions()).definitions
        )
        by = (
            ["asset_class"]
            if "asset_class" in getattr(known.get("market_value"), "breakdowns", ())
            else []
        )
        job = analytics.job(analytics.measure("market_value", by=by), accounts=accounts)
        return await self.jobs.run(self.plugin, job)

    async def lake_rows(self, subject: str) -> ops.QueryLakeResult:
        """The closes the lake holds of one subject, its price alone, at most
        100 rows: one bounded query, cut and said so past the limit."""
        return await self.plugin.query_lake(
            kind="prices",
            subjects=[ops.SubjectRef(entity_id=subject)],
            fields=["meridian.v1.Price.price"],
            row_limit=100,
        )

    # ── Rates and execution costs (contract v27) ──────────────────────

    async def curve(self, subject: str, day: date) -> list[ops.Rate]:
        """A rate curve's points for a business date, in tenor order, from
        the deployment's default sources."""
        return await rates.read_curve(self.plugin, subject, day)

    async def costs(self, account: str, day: date) -> list[ops.ExecutionCost]:
        """An account's execution costs for a trade date, against arrival
        and the other standard benchmarks, every page read."""
        out: list[ops.ExecutionCost] = []
        cursor = ""
        while True:
            page = await self.plugin.list_execution_costs(
                account_id=account, trade_date=day, cursor=cursor
            )
            out.extend(page.costs)
            cursor = page.next_cursor
            if not cursor:
                return out

    async def on_rate(self, heard: meridian.Heard[ops.RatesRecordedEvent]) -> None:
        """A rate the lake recorded, for a subject this plugin reads."""
        self.rates_heard += 1
        self.last_heard = heard.message.rate.meta.source.dataset

    # ── Heard ───────────────────────────────────────────────────────────

    async def on_price(self, heard: meridian.Heard[ops.PricesRecordedEvent]) -> None:
        """A price the lake recorded, for a subject this plugin reads."""
        self.prices_heard += 1
        self.last_heard = heard.message.price.meta.source.dataset

    async def on_bar(self, heard: meridian.Heard[ops.BarsRecordedEvent]) -> None:
        """A bar the lake recorded, for a subject this plugin reads."""
        self.bars_heard += 1
        self.last_heard = heard.message.bar.meta.source.dataset
