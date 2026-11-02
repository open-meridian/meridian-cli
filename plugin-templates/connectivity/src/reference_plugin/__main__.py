"""A Meridian connectivity plugin, as `meridian plugin new --role connectivity`
writes one.

It runs the SDK's Trader over its venue (venue.py, a stand-in here) -- the
connectivity role's class -- on the sidecar it is launched beside, declaring
its settings and its pages: placements heard and sent at most once, what the
venue did reported, fills into the book, the firm's pauses and stops kept,
until it is stopped. What it notes so a restart sends nothing twice is kept
in the storage the deployment grants it.
"""

import logging
from datetime import UTC, datetime
from zoneinfo import ZoneInfo

import meridian
from meridian import edge
from meridian.roles import connectivity

from .declaration import DECLARATION, PORT
from .page import pages, running
from .venue import StandInVenue

#: The listing market's zone: the day a fill's trade date is in.
MARKET_ZONE = ZoneInfo("America/New_York")


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    venue = StandInVenue()
    sent, enabled, originated = connectivity.trading_files(edge.storage_dir())
    trader = connectivity.Trader(
        lambda: venue,
        now=lambda: datetime.now(UTC),
        sent=connectivity.Sent(sent),
        enabled=connectivity.Sent(enabled),
        watched=connectivity.Watched(originated),
        market_zone=MARKET_ZONE,
        poll_seconds=lambda: 5.0,
    )
    running["trader"] = trader
    meridian.run(declaration=DECLARATION, roles=[trader], pages=pages, port=PORT)


if __name__ == "__main__":
    main()
