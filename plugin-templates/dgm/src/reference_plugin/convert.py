"""The conversion: what the vendor said, put into the lake in the contract's
words, once, at the edge.

Every number is parsed from the vendor's own text as a `Decimal`, never
through a `float` (`json.loads(..., parse_float=Decimal)`): a binary float
holds about 17 significant digits, so a price with more decimal places than
that would be recorded as some other price, and the SDK refuses a float
outright. Every subject and venue is resolved before a row names it, and
what does not resolve is reported, never guessed: the plugin never mints
reference data. Each row's key is made from its raw record, so a repeat
changes nothing and the day still forming is restated under the same key,
its business date the day the dataset's declared day says it is. A price in
a token with no ISO 4217 code is in the token's own cash instrument.

`meridian.suites.run("dgm", producers)` holds this to the role's suite
(tests/test_suite.py): each case mapped to one of the vendor's responses,
run through this code, against a recorder standing in for the sidecar.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import UTC, date, datetime, timedelta
from decimal import Decimal
from typing import Any

import meridian
from meridian.plugin.v1 import operations_pb2 as ops

from .declaration import DAILY
from .vendor import FIAT, Vendor

#: How the vendor is named on what this plugin reports: its identifiers'
#: `source`, and its misses'.
SOURCE = "reference"

#: The most rows one RecordPrices or RecordBars takes.
BATCH = 500

NS = 1_000_000_000


def parse(text: str) -> dict[str, Any]:
    """The vendor's JSON, every number a Decimal as the vendor wrote it."""
    parsed: dict[str, Any] = json.loads(text, parse_float=Decimal, parse_int=Decimal)
    return parsed


def _ns(moment: datetime) -> int:
    return int(moment.timestamp()) * NS + moment.microsecond * 1_000


def _as_of(want: ops.ObservationsWantedEvent) -> int:
    """The moment a want asks about, to read a record as of (W3.6): its
    valid time, else the start of its business date, else now."""
    if want.valid_from_ns:
        return int(want.valid_from_ns)
    if want.business_date:
        return _ns(datetime.fromisoformat(want.business_date).replace(tzinfo=UTC))
    return _ns(datetime.now(UTC))


@dataclass
class Converter:
    """The plugin's conversion, against whatever operations it is given: the
    plugin's own when it runs, a recorder's in its tests."""

    plugin: meridian.Plugin
    vendor: Vendor = field(default_factory=Vendor)
    #: The vendor's symbols by the instrument each resolved to: what a want,
    #: which names instruments, is served from. A plugin owning storage keeps
    #: this map there, never a vendor's code as a key in core.
    products: dict[str, str] = field(default_factory=dict)
    #: The standing wants it keeps current, by want: their subjects.
    standing: dict[str, tuple[str, ...]] = field(default_factory=dict)

    @property
    def dataset(self) -> str:
        """The dataset its rows name: the instance, a colon and the key."""
        return f"{self.plugin.identity.instance_id}:{DAILY.key}"

    # ── Resolving ───────────────────────────────────────────────────────

    async def instrument(self, symbol: str, as_of_ns: int) -> str:
        """The deployment's instrument for the vendor's symbol, as of a date;
        empty, with the miss reported, where it does not resolve to one."""
        named = [meridian.Identifier(scheme="symbol", value=symbol, source=SOURCE)]
        found = await self.plugin.resolve_identifier(identifiers=named, as_of_ns=as_of_ns)
        if found.found and found.instrument_id:
            return str(found.instrument_id)
        await self.plugin.report_missing_instrument(
            source=SOURCE,
            identifiers=named,
            as_of_ns=as_of_ns,
            reason=found.miss_reason or ops.MISS_REASON_NOT_FOUND,
        )
        return ""

    async def venue(self, mic: str, as_of_ns: int) -> str:
        """The venue master's ID for the MIC the vendor names; empty, with
        the miss reported, where the deployment holds no such venue."""
        named = [meridian.Identifier(scheme="iso10383", value=mic)]
        found = await self.plugin.resolve_venue(identifiers=named, as_of_ns=as_of_ns)
        if found.found and found.venue.venue_id:
            return str(found.venue.venue_id)
        await self.plugin.report_missing_venue(
            source=SOURCE,
            identifiers=named,
            as_of_ns=as_of_ns,
            reason=found.miss_reason or ops.MISS_REASON_NOT_FOUND,
        )
        return ""

    async def map_products(self, as_of_ns: int) -> None:
        """Resolve each symbol the vendor serves this key, so a want naming
        its instrument can be served."""
        for symbol in self.vendor.entitled:
            text = self.vendor.daily(symbol)
            if text is None:
                continue
            instrument = await self.instrument(str(parse(text)["base"]), as_of_ns)
            if instrument:
                self.products[instrument] = symbol

    async def symbol_for(self, instrument: str, as_of_ns: int) -> str | None:
        """The vendor's symbol for an instrument a want names. One this
        plugin resolved is in `products`; one another source resolved -- a
        custodian's, say -- is read from its record (W3.6) and matched by a
        symbol in it to one the vendor serves, then kept. None where nothing
        in the record is one the vendor serves."""
        known = self.products.get(instrument)
        if known is not None:
            return known
        found = await self.plugin.resolve_instrument(
            instrument_id=instrument, as_of_ns=as_of_ns
        )
        if not found.found:
            return None
        named = {i.value for i in found.instrument.identifiers if i.scheme == "symbol"}
        for symbol in self.vendor.entitled:
            text = self.vendor.daily(symbol)
            if text is not None and str(parse(text)["base"]) in named:
                self.products[instrument] = symbol
                return symbol
        return None

    # ── Converting ──────────────────────────────────────────────────────

    async def convert(
        self, symbol: str, text: str
    ) -> tuple[list[meridian.Price], list[meridian.Bar]]:
        """One daily candle, as the vendor sent it, as the lake's rows: its
        close, and its bar where the vendor states the day's range."""
        candle = parse(text)
        day = date.fromisoformat(str(candle["day"]))
        # The dataset's declared day: Etc/UTC, ending at midnight.
        start = datetime(day.year, day.month, day.day, tzinfo=UTC)
        as_of = _ns(start)
        subject = await self.instrument(str(candle["base"]), as_of)
        if not subject:
            return [], []
        quote = str(candle["quote"])
        if quote in FIAT:
            currency = {"currency_code": quote}
        else:
            # A token has no ISO 4217 code: its price is in its own cash
            # instrument, never in a fiat code it is pegged to.
            token = await self.instrument(quote, as_of)
            if not token:
                return [], []
            currency = {"instrument_id": token}
        venue = await self.venue(str(candle["mic"]), as_of) if "mic" in candle else ""
        meta = meridian.ObservationMeta(
            row_key=f"{symbol}:1d:{day.isoformat()}",
            subjects=[meridian.SubjectRef(entity_id=subject)],
            source=meridian.Source(dataset=self.dataset, venue_id=venue),
            valid_from_ns=as_of,
            valid_until_ns=_ns(start + timedelta(days=1)),
            business_date=day,
            source_times=[
                meridian.SourceTime(
                    kind="published",
                    at_ns=_ns(datetime.fromisoformat(str(candle["published"]))),
                )
            ],
            raw=self.plugin.raw_record(f"daily/{symbol}/{day.isoformat()}"),
        )

        def money(amount: Decimal) -> meridian.Money:
            return meridian.Money(amount, **currency)

        prices = [
            meridian.Price(
                meta=meta, kind="close", price=money(candle["close"]), basis="per_unit"
            )
        ]
        bars = []
        if {"open", "high", "low"} <= candle.keys():
            bars.append(
                meridian.Bar(
                    meta=meta,
                    open=money(candle["open"]),
                    high=money(candle["high"]),
                    low=money(candle["low"]),
                    close=money(candle["close"]),
                    volume=candle.get("volume", Decimal(0)),
                )
            )
        return prices, bars

    async def record(
        self, prices: list[meridian.Price], bars: list[meridian.Bar], want_id: str = ""
    ) -> None:
        """Rows recorded in batches of at most BATCH, against a want where
        they answer one."""
        for at in range(0, len(prices), BATCH):
            await self.plugin.record_prices(prices=prices[at : at + BATCH], want_id=want_id)
        for at in range(0, len(bars), BATCH):
            await self.plugin.record_bars(bars=bars[at : at + BATCH], want_id=want_id)

    async def record_daily(self, symbol: str, text: str) -> None:
        """A candle the vendor sent, recorded: the push path."""
        prices, bars = await self.convert(symbol, text)
        await self.record(prices, bars)

    # ── Wants ───────────────────────────────────────────────────────────

    async def on_want(self, heard: meridian.Heard[ops.ObservationsWantedEvent]) -> None:
        """What the lake wants of this plugin's datasets: recorded against,
        where the vendor serves the subject, and declined where it does not."""
        want = heard.message
        prices: list[meridian.Price] = []
        bars: list[meridian.Bar] = []
        declined = []
        as_of_ns = _as_of(want)
        for subject in want.subjects:
            symbol = await self.symbol_for(subject.entity_id, as_of_ns)
            text = self.vendor.daily(symbol) if symbol else None
            if symbol is None or text is None:
                declined.append(subject)
                continue
            more_prices, more_bars = await self.convert(symbol, text)
            prices += more_prices
            bars += more_bars
        if prices or bars:
            await self.record(prices, bars, want_id=want.want_id)
        if declined:
            await self.plugin.decline_want(
                want_id=want.want_id, subjects=declined, reason="not_covered"
            )
        if want.standing:
            self.standing[want.want_id] = tuple(s.entity_id for s in want.subjects)

    async def on_withdrawn(self, heard: meridian.Heard[ops.WantWithdrawnEvent]) -> None:
        """A standing want no reader asked for within the dataset's cadence:
        no longer kept current."""
        self.standing.pop(heard.message.want_id, None)
