"""A data plugin's common part (spec/plugin-pages-follow-their-role, "data, a
data plugin"), its restart's backfill (contract v20, W10.6), and from 0.30.0
the logic every data plugin shares whatever its venue (sdk-contract/the-sdk-
carries-each-roles-common-logic, ruled 2026-10-13).

**First-use path.** Enter the key (Settings); check the connection
(Connection); have its datasets licensed and entitled (a deployment admin, on
Data sources); see data arrive (Datasets). Prices are seen in a reading
plugin's pages, never here.

**Settings**, in this order: `meridian.environment()` where the vendor has
more than one; the credentials, each a required secret labelled in the
vendor's words and gated on the environment where they differ by it; the
vendor's own; `meridian.sample_data()`.

**Pages**, Manage only, in this order:

    @pages.page("/", "Connection", levels="admin", answers=data.ConnectionRead,
                name="read_connection")
    async def connection(request):
        return pages.answer("meridian/roles/data/connection.html", ConnectionRead(...))

    @pages.page("/datasets", "Datasets", levels="admin", answers=data.DatasetsRead,
                name="read_datasets")

Connection's sections are "Key", "Limits and use" and "Stream"; Datasets'
"Served", "Wants" (searched and paged, since they grow) and "Subjects". From
contract v20 a data plugin reads its own declared datasets' licences
(`plugin.list_datasets()`, its `licences`), so Datasets says "Licensed" or
"Not licensed yet" exactly.

**Figures**, the role's first: "Connection", "Entitled at the vendor",
"Stream", "Last recorded", "Rate use" (`figures(...)`); three places remain
for the vendor's.

**Wants, standing wants and recording** (`Wants`, W10.4, W10.7, W10.9): what
the lake wants heard and answered by the plugin's own conversion, what it
declines declined by reason, a standing want kept until withdrawn, rows
sent in the lake's batches and counted overall and per dataset, its
datasets' licences read; `served` and `want_rows` fill Datasets from it.
`Venue` resolves the venue master's ID once per reading (W3.14), reporting a
venue not held (W3.15); `unconverted` carries a value that did not read, and
a venue not held, as reported.

**A restart's gap** (`backfill_restart`, or `Wants.backfill` once per subject
per process): on start, before resuming its stream, a data plugin that
records trades reads the last trade its own instance recorded of each
subject it streams (`list_trades(..., latest_recorded=True)`), reads the
venue's history from each, and records what it missed; a trade recorded
again is answered unchanged by its row key, so the overlap costs nothing.

**Reading a venue** (`number`, `json_decimal`, `Unread`, `utc_day`): every
figure a `Decimal` from the venue's own text, never a float, and a value
that would not read kept as sent. `Session`, `Health`, `Limits` and
`Endpoint` say what the plugin connects to and how it is going, for
Connection (`VenueConnection`, a source of one venue).

**A venue's whole work, by what it lists** (0.30.0), generic over the venue,
the plugin supplying its client and conversion (a `Market`) and, where it
streams, its transport:

- `meridian.roles.data.pairs` -- a venue listing pairs, a base asset priced
  in a quote asset (a crypto exchange's products): daily closes and bars,
  live prices, trades and quotes, the stream kept current and its gaps read
  back;
- `meridian.roles.data.event_contracts` -- a venue of binary event
  contracts: live prices and quotes per contract, and each contract's
  outcome (contract v23); from contract v26 the venue's event groups
  reported (`Feed.report_event`); from 0.33.1 its hourly and daily bars,
  where its `Market` answers `bars` (`Feed.keep_bars`);
- `meridian.roles.data.listings` (0.31.0, ruling 213) -- a broker's US
  equity listings: daily closes and bars, live prices and quotes, trades
  streamed and, where the broker reads its history, a range and a gap read
  back; the plugin subclasses `Feed`, naming its source and converting its
  broker's trade conditions;
- `meridian.roles.data.rates` (0.32.0, contract v27; ruling 345) -- the
  lake's rates slice: a publisher's fixings, averages, index levels and
  curves' points as `Rate` rows, subjects resolved by the identifiers it
  states, a same-day revision the next version of its row key, a tenor not
  published no row; and a reader's curve for a date (`read_curve`);
- `meridian.roles.data.testing` -- the lake as a test stands it in, and the
  data suite's producers for each, so a plugin runs the suite unchanged
  against its own conversion.

- `meridian.roles.data.filings` (0.33.0, contract v28) -- the lake's
  filings slice: filings, reported facts, holdings reports and holdings,
  declared dividends and rating actions, each under its company or
  security with its role, companies resolved by the identifiers stated
  (`Companies`);

**Quality states** (0.33.0, contract v28, ruling 424): every row recorded
unasked or for a standing want is judged at the edge
(`meridian.roles.data.quality`, `Wants.quality`) -- a crossed quote
invalid, a duplicate, the rows after a sequence gap, a row out of order
or stale suspicious, a source time ahead of the clock invalid -- its state
and reasons on its envelope; the tolerances the admin's
`dataset_thresholds`. `Wants.record` takes a chain's series and
statistics and the filings slice's rows too.

**Latency and liveness** (0.32.0, contract v27): given the plugin's
`roles.Latency` (`Wants.latency`), every row recorded unasked or for a
standing want has its age at the `edge` hop recorded, its feed delay,
reported from 0.33.0 (`Latency.report`; the suite's
`reports-its-feed-delay` is required from contract v28, ruling 518); a
stream's heartbeat with no price
changed, or a poll, is the dataset's liveness (`Wants.liveness`,
`RecordLiveness`), which confirms the values in force; Connection's
"Sessions" section shows each session's `SessionLatency`
(`ConnectionRead.session_latency`), drawn only once a session is measured
(0.32.2): a polled plugin's Connection has none. A dataset declares its delay
(`DatasetDeclaration.declared_delay_ns`) and its admin its staleness
(`meridian.dataset_thresholds()`), and the plugin admin its ceilings
(`meridian.session_ceilings()`).

How a plugin talks to its venue -- an API client, a WebSocket, FIX -- is its
own, never here.
"""

from __future__ import annotations

from ...figures import Figure
from .._figures import Value, ordered
from ._pages import served, want_rows
from ._reading import (
    NANOS_PER_DAY,
    NANOS_PER_SECOND,
    SECONDS_PER_DAY,
    Endpoint,
    Health,
    Limits,
    Session,
    Unread,
    day_start,
    iso_time,
    json_decimal,
    number,
    utc_day,
)
from ._records import (
    ConnectionRead,
    Credential,
    DatasetsRead,
    Served,
    VenueConnection,
    Want,
    WantHeard,
)
from ._wants import (
    BATCH,
    Counts,
    Heard,
    History,
    Standing,
    Venue,
    Wants,
    asked,
    backfill_restart,
    cancelled,
    same,
    unconverted,
)

__all__ = [
    "BATCH",
    "NANOS_PER_DAY",
    "NANOS_PER_SECOND",
    "SECONDS_PER_DAY",
    "ConnectionRead",
    "Counts",
    "Credential",
    "DatasetsRead",
    "Endpoint",
    "Health",
    "Heard",
    "History",
    "Limits",
    "Served",
    "Session",
    "Standing",
    "Unread",
    "Venue",
    "VenueConnection",
    "Want",
    "WantHeard",
    "Wants",
    "asked",
    "backfill_restart",
    "cancelled",
    "day_start",
    "figures",
    "iso_time",
    "json_decimal",
    "number",
    "same",
    "served",
    "unconverted",
    "utc_day",
    "want_rows",
]


def _connection(value: object) -> str | None:
    return {"Not answering": "error", "No key": "warn"}.get(str(value))


def figures(
    *,
    connection: Value,
    entitled_at_vendor: Value = None,
    stream: Value = None,
    last_recorded: Value = None,
    rate_use: Value = None,
) -> list[Figure]:
    """The data role's figures, in its order, each a plain value or a
    `meridian.Figure` carrying its state and why: "Connection" ("Answering",
    "Not answering", an error, "No key", a warning, or "Sample data");
    "Entitled at the vendor" ("3 of 4 datasets", warn when fewer than
    declared, why naming those not served); "Stream" ("12 listings", "Off" or
    "Does not stream"); "Last recorded" (a time, error when the last
    recording failed); "Rate use" ("41 of 100 a minute", warn at four fifths
    of the limit). None leaves one out. Follow them with the vendor's own."""
    return ordered(
        [
            ("Connection", connection, _connection),
            ("Entitled at the vendor", entitled_at_vendor, None),
            ("Stream", stream, None),
            ("Last recorded", last_recorded, None),
            ("Rate use", rate_use, None),
        ]
    )
