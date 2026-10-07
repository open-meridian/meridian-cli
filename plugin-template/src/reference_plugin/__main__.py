"""A Meridian plugin, as `meridian plugin new` writes one.

It connects to the sidecar it is launched beside, declaring its pages, says
who it was launched as and what the deployment lets it do, serves the pages
people reach through the dashboard (page.py), reports itself healthy, and runs
until it is stopped. Heartbeats are sent for it.

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

from .page import REACHES, TITLE, pages, reports

log = logging.getLogger("reference_plugin")


async def run() -> None:
    # Before anything else: a stop that arrives while connecting is still a
    # stop, and without these the default handler kills the process before it
    # has left its sidecar cleanly.
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    # Where the pages listen, on loopback beside the sidecar, which forwards
    # people's requests to them. Each page is declared with the levels it
    # serves, and the dashboard shows it under those buttons.
    port = int(os.environ.get("REFERENCE_PAGE_PORT", "8000"))
    async with await meridian.connect(
        interface=meridian.Interface(port=port, title=TITLE, pages=pages)
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
        # The external accounts its connection reaches, which an admin of
        # the plugin then links on Setup: what it records is for those. Only
        # where its roles let it; a plugin holding none reaches nothing.
        if reports(plugin):
            await plugin.report_external_accounts(accounts=REACHES)
            log.info("reported %d external account(s) to link", len(REACHES))
        served = pages.serve(plugin, port)
        log.info("serving its pages on 127.0.0.1:%d", port)
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        log.info("stopping")
        served.shutdown()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
