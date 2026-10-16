"""A Meridian custody plugin, as `meridian plugin new --role custody` writes one.

It connects to the sidecar it is launched beside, declaring its settings and
its pages, reports the external accounts its custodian reaches, reports the
custody role's figures, and serves its pages until it is stopped. Heartbeats
are sent for it.
"""

import asyncio
import logging
import signal

import meridian
from meridian.roles import custody

from .custodian import Custodian
from .declaration import DECLARATION, PORT, Config
from .page import pages, use

log = logging.getLogger("reference_plugin")


async def run() -> None:
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    async with await meridian.connect(declaration=DECLARATION) as plugin:
        log.info("registered as %s", plugin.identity.instance_id)
        custodian = Custodian()
        config = await plugin.first_settings(Config)
        use(custodian, key_set=bool(config.api_key))

        async def follow_scope() -> None:
            # The account scope and this plugin's links, on start and on every
            # change: the pages hold them from here, never the plugin's own.
            async for scope in plugin.account_scope():
                use(custodian, key_set=bool(config.api_key), scope=scope)

        following = asyncio.create_task(follow_scope())
        plugin.figures = custody.figures(
            connections=1, accounts_reached=len(custodian.accounts), last_read="Not yet"
        )
        served = pages.serve(plugin, PORT)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        following.cancel()
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
