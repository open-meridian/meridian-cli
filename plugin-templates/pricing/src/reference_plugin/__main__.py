"""A Meridian `pricing` plugin, as `meridian plugin new --role pricing` writes
one.

It connects to the sidecar it is launched beside, declaring its page, and
runs the SDK's Pricer over its models (model.py): the batches still waiting
after the last sequence it answered, then each batch routed to it, answered
whole, until it is stopped. The last sequence answered is kept in the
storage the deployment grants it.
"""

import asyncio
import logging
import signal

import meridian
from meridian import edge
from meridian.roles import pricing

from .declaration import DECLARATION, PORT
from .model import MODELS
from .page import pages, running

log = logging.getLogger("reference_plugin")


async def run() -> None:
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    async with await meridian.connect(declaration=DECLARATION) as plugin:
        log.info("registered as %s", plugin.identity.instance_id)
        granted = edge.storage_dir()
        kept = pricing.Sequence(granted / "pricing.json" if granted is not None else None)
        pricer = pricing.Pricer(plugin, MODELS, sequence=kept)
        running["pricer"] = pricer
        answering = asyncio.create_task(pricer.run())
        served = pages.serve(plugin, PORT)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        answering.cancel()
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
