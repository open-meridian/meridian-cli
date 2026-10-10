"""A Meridian `reporting` plugin, as `meridian plugin new --role reporting`
writes one.

It connects to the sidecar it is launched beside, declaring its pages, reads
the datasets it may read and the positions in its account scope, then hears
what the lake records for the instruments held. Its pages read the book and
the lake as people open them, and record nothing. It reports itself healthy
and runs until it is stopped. Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential,
knows no other address, and cannot choose its own identity or grants: those
come from how the deployment launched it.
"""

import asyncio
import logging
import os
import signal

import meridian

from .page import TITLE, last_close, pages, use
from .report import Report

log = logging.getLogger("reference_plugin")


async def run() -> None:
    # Before anything else: a stop that arrives while connecting is still a
    # stop, and without these the default handler kills the process before it
    # has left its sidecar cleanly.
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    port = int(os.environ.get("REFERENCE_PAGE_PORT", "8000"))
    async with await meridian.connect(
        interface=meridian.Interface(port=port, title=TITLE, pages=pages)
    ) as plugin:
        log.info(
            "registered as %s, roles %s",
            plugin.identity.instance_id,
            ", ".join(plugin.identity.roles) or "none",
        )
        report = Report(plugin)
        use(report)
        log.info("may read %s", ", ".join(await report.datasets()) or "no dataset yet")
        # What the lake records is heard for the subjects named: here the
        # instruments held at the last close. A plugin following its book
        # names them again as positions change.
        held = sorted({p.instrument_id for p in await report.positions(last_close())})
        hearing = None
        if held:
            hearing = asyncio.create_task(
                plugin.receive(
                    prices_recorded=report.on_price, bars_recorded=report.on_bar, subjects=held
                )
            )
        served = pages.serve(plugin, port)
        log.info("serving its pages on 127.0.0.1:%d", port)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        log.info("stopping")
        if hearing is not None:
            hearing.cancel()
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
