"""What this plugin reports: the book's positions at a day's close, each with
the close the lake holds for it, and the change over the week to that day.

A `reporting` plugin reads the book and the lake and relates their records;
it never records anything back, and never matches orders. What it computes
-- here a position's value at the close -- is computed and shown, never
recorded. It reads prices by the deployment's source priority, which a
deployment admin sets on the dashboard's Data sources page, from the
datasets it is entitled to read.

Every number stays a `Decimal`, as the lake keeps it: never a `float`.

`meridian.suites.run("reporting", producers)` holds this to the role's
suite (tests/test_suite.py), against a recorder standing in for the sidecar.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, date, datetime, timedelta
from decimal import Decimal

import meridian
from meridian.plugin.v1 import operations_pb2 as ops

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
    #: What the lake has recorded since the plugin started, as heard.
    prices_heard: int = 0
    bars_heard: int = 0
    last_heard: str = ""
    datasets_read: list[str] = field(default_factory=list)

    async def datasets(self) -> list[str]:
        """The datasets this plugin may read, by name."""
        read = await self.plugin.list_datasets()
        self.datasets_read = [ref.dataset for ref in read.datasets]
        return self.datasets_read

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

    # ── Heard ───────────────────────────────────────────────────────────

    async def on_price(self, heard: meridian.Heard[ops.PricesRecordedEvent]) -> None:
        """A price the lake recorded, for a subject this plugin reads."""
        self.prices_heard += 1
        self.last_heard = heard.message.price.meta.source.dataset

    async def on_bar(self, heard: meridian.Heard[ops.BarsRecordedEvent]) -> None:
        """A bar the lake recorded, for a subject this plugin reads."""
        self.bars_heard += 1
        self.last_heard = heard.message.bar.meta.source.dataset
