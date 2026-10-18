"""A Meridian connectivity plugin, as `meridian plugin new --role connectivity`
writes one.

It connects to the sidecar it is launched beside, declaring its settings and
its pages, and runs the SDK's Trader over its venue (venue.py, a stand-in
here): placements heard and sent at most once, what the venue did reported,
fills into the book, the firm's pauses and stops kept, until it is stopped.
What it notes so a restart sends nothing twice is kept in the storage the
deployment grants it.
"""

import asyncio
import logging
import signal
from datetime import UTC, datetime
from zoneinfo import ZoneInfo

import meridian
from meridian import edge
from meridian.roles import connectivity

from .declaration import DECLARATION, PORT
from .page import pages, running
from .venue import StandInVenue

log = logging.getLogger("reference_plugin")

#: The listing market's zone: the day a fill's trade date is in.
MARKET_ZONE = ZoneInfo("America/New_York")


async def run() -> None:
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    async with await meridian.connect(declaration=DECLARATION) as plugin:
        log.info("registered as %s", plugin.identity.instance_id)
        venue = StandInVenue()
        sent, enabled, originated = connectivity.trading_files(edge.storage_dir())
        trader = connectivity.Trader(
            plugin,
            lambda: venue,
            now=lambda: datetime.now(UTC),
            sent=connectivity.Sent(sent),
            enabled=connectivity.Sent(enabled),
            watched=connectivity.Watched(originated),
            market_zone=MARKET_ZONE,
        )
        running["trader"] = trader
        trading = asyncio.create_task(trader.run(poll_seconds=lambda: 5.0))
        served = pages.serve(plugin, PORT)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        trading.cancel()
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
