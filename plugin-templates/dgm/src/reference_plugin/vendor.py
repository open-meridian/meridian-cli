"""A stand-in for the vendor this plugin reads: replace it with your own.

A real `dgm` reads its vendor's API, a file it publishes, or a feed, with
whichever library suits it: the SDK carries no HTTP or WebSocket client and
no parser of a vendor's formats, so the choice is the plugin's. What it hands
the conversion (convert.py) is what the vendor said, as text, exactly as it
came: the conversion parses it, once, at the edge.

This one answers from the responses below, written the way a vendor writes
JSON, prices as JSON numbers. It reaches nothing, so the plugin runs anywhere,
and its tests map each case of the `dgm` suite to one of its responses.
"""

from __future__ import annotations

from dataclasses import dataclass, field

#: What the vendor calls itself, as the catalogue shows it.
VENDOR = "Reference vendor"

#: The currencies the vendor quotes in, as its own list of them says: a fiat
#: currency by its ISO 4217 code, anything else a token with no such code.
FIAT = frozenset({"USD", "EUR", "GBP", "JPY"})

#: Each day's candle by the vendor's symbol, as its daily endpoint answers:
#: what it prices and in what, the day, its open, high, low and close where
#: it states them, its volume, the venue's MIC where it names one, and when
#: it published the candle.
DAILY: dict[str, str] = {
    "BTC-USD": (
        '{"base": "BTC", "quote": "USD", "day": "2026-10-08", "open": 61000,'
        ' "high": 62900, "low": 60500.5, "close": 62431.27, "volume": 1234.56789012,'
        ' "published": "2026-10-09T00:00:00Z"}'
    ),
    "SHIB-USD": (
        '{"base": "SHIB", "quote": "USD", "day": "2026-10-08",'
        ' "close": 0.000012345678901234, "published": "2026-10-09T00:00:00Z"}'
    ),
    "EUR-USD": (
        '{"base": "EUR", "quote": "USD", "day": "2026-10-08", "close": 1.0921,'
        ' "published": "2026-10-09T00:00:00Z"}'
    ),
    "ETH-USDC": (
        '{"base": "ETH", "quote": "USDC", "day": "2026-10-08", "close": 2410.5,'
        ' "published": "2026-10-09T00:00:00Z"}'
    ),
    "QQQ": (
        '{"base": "QQQ", "quote": "USD", "day": "2026-10-08", "close": 501.2,'
        ' "mic": "XNAS", "published": "2026-10-08T20:00:00Z"}'
    ),
}

#: BTC-USD's candle while its day is still forming, read twice: the vendor's
#: second answer replaces its first.
FORMING: tuple[str, str] = (
    '{"base": "BTC", "quote": "USD", "day": "2026-10-08", "close": 62000.1,'
    ' "published": "2026-10-08T12:00:00Z"}',
    '{"base": "BTC", "quote": "USD", "day": "2026-10-08", "close": 62431.27,'
    ' "published": "2026-10-09T00:00:00Z"}',
)


@dataclass
class Vendor:
    """The vendor's API, as this plugin calls it. `calls` counts what it was
    asked and `failed` what went wrong last, for the Connection page: a real
    client sets it when a call fails, and clears it when one succeeds."""

    #: The calls a minute the vendor's terms allow this plugin's key.
    limit_per_minute: int = 60
    calls: int = 0
    failed: str = ""
    #: The symbols the vendor serves this plugin's key: what it is entitled to.
    entitled: tuple[str, ...] = field(default_factory=lambda: tuple(DAILY))

    def daily(self, symbol: str) -> str | None:
        """The vendor's daily candle for `symbol`, as it sent it; None where
        it serves no such symbol to this key."""
        self.calls += 1
        return DAILY.get(symbol) if symbol in self.entitled else None
