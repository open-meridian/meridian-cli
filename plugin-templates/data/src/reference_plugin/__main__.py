"""A Meridian data plugin, as `meridian plugin new --role data` writes one.

It connects to the sidecar it is launched beside, declaring its settings,
its catalogue and its pages, reads which of its datasets a deployment admin
has licensed, reports the data role's figures, resolves the symbols its
vendor serves it, then hears what the
lake wants of its datasets and records against each want from its vendor, or
declines what the vendor does not serve. It reports itself healthy and runs
until it is stopped. Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential
of the deployment's, knows no other address of it, and cannot choose its own
identity or grants: those come from how the deployment launched it. Its
vendor is its own to reach (vendor.py stands in for it here), with a key in a
secret setting where the vendor needs one.
"""

import asyncio
import logging
import signal
import time

import meridian
from meridian.roles import data

from .convert import Converter
from .declaration import DECLARATION, PORT, Config
from .page import pages, use

log = logging.getLogger("reference_plugin")


async def run() -> None:
    # Before anything else: a stop that arrives while connecting is still a
    # stop, and without these the default handler kills the process before it
    # has left its sidecar cleanly.
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    async with await meridian.connect(declaration=DECLARATION) as plugin:
        log.info(
            "registered as %s, roles %s",
            plugin.identity.instance_id,
            ", ".join(plugin.identity.roles) or "none",
        )
        converter = Converter(plugin)
        use(converter)

        def configure(config: Config) -> None:
            converter.sample_data = config.sample_data

        configuring = asyncio.create_task(plugin.on_settings(Config, configure))
        await converter.read_licences()
        plugin.figures = data.figures(
            connection="Sample data" if converter.sample_data else "Answering",
            entitled_at_vendor=f"{len(DECLARATION.catalogue)} of "
            f"{len(DECLARATION.catalogue)} datasets",
            stream="Does not stream",
            rate_use=f"{converter.vendor.calls} of "
            f"{converter.vendor.limit_per_minute} a minute",
        )
        await converter.map_products(time.time_ns())
        log.info("resolved %d of the vendor's symbols", len(converter.products))
        # The lake's wants, heard for as long as the plugin runs: each
        # recorded against or declined. The stream stays open.
        hearing = asyncio.create_task(
            plugin.receive(
                observations_wanted=converter.on_want, want_withdrawn=converter.on_withdrawn
            )
        )
        served = pages.serve(plugin, PORT)
        log.info("serving its pages on 127.0.0.1:%d", PORT)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        log.info("stopping")
        hearing.cancel()
        configuring.cancel()
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
