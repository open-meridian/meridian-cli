"""A Meridian plugin, as `meridian plugin new` writes one.

It connects to the sidecar it is launched beside, says who it was launched as
and what the deployment lets it do, serves a page people reach through the
dashboard (page.py), reports itself healthy, and runs until it is stopped.
Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential,
knows no other address, and cannot choose its own identity or grants: those
come from how the deployment launched it. Start from here -- the typed
operations on `plugin` are the steps its roles may take, and nothing else
reaches the bus -- and keep it that way.
"""

import asyncio
import logging
import os
import signal

import meridian

from .page import TITLE, serve

log = logging.getLogger("reference_plugin")


async def run() -> None:
    # Before anything else: a stop that arrives while connecting is still a
    # stop, and without these the default handler kills the process before it
    # has left its sidecar cleanly.
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    # Where the page listens, on loopback beside the sidecar, which forwards
    # people's requests to it.
    port = int(os.environ.get("REFERENCE_PAGE_PORT", "8000"))
    async with await meridian.connect(
        interface=meridian.Interface(port=port, title=TITLE)
    ) as plugin:
        log.info(
            "registered as %s, roles %s",
            plugin.identity.instance_id,
            ", ".join(plugin.identity.roles) or "none",
        )
        log.info(
            "may publish %s; may subscribe %s",
            ", ".join(plugin.grants.publish) or "nothing",
            ", ".join(plugin.grants.subscribe) or "nothing",
        )
        page = serve(plugin, loop, port)
        log.info("serving its page on 127.0.0.1:%d", port)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        log.info("stopping")
        page.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
