"""A Meridian operations plugin, as `meridian plugin new --role operations`
writes one.

It connects to the sidecar it is launched beside, declaring its settings and
its pages, follows its settings, reports the operations role's figures, and
serves its pages until it is stopped. Heartbeats are sent for it.
"""

import asyncio
import logging
import signal

import meridian
from meridian.roles import operations

from .declaration import DECLARATION, PORT, Config
from .page import pages, running

log = logging.getLogger("reference_plugin")


async def run() -> None:
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    async with await meridian.connect(declaration=DECLARATION) as plugin:
        log.info("registered as %s", plugin.identity.instance_id)

        def configure(config: Config) -> None:
            running["due_business_days"] = config.due_business_days

        configuring = asyncio.create_task(plugin.on_settings(Config, configure))
        plugin.figures = operations.figures(
            open_breaks=0, overdue_breaks=0, last_reconciliation="Not yet"
        )
        served = pages.serve(plugin, PORT)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        configuring.cancel()
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
