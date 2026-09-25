"""A Meridian plugin, as `meridian plugin new` writes one.

It connects to the sidecar it is launched beside, says who it was launched as
and what the deployment lets it do, reports itself healthy, and runs until it
is stopped. Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential,
knows no other address, and cannot choose its own identity or grants: those
come from how the deployment launched it. Start from here -- subscribe to what
you were granted, publish what you produce -- and keep it that way.
"""

import asyncio
import logging
import signal

import meridian

log = logging.getLogger("reference_plugin")


async def run() -> None:
    # Before anything else: a stop that arrives while connecting is still a
    # stop, and without these the default handler kills the process before it
    # has left its sidecar cleanly.
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(stop, stopped.set)

    async with await meridian.connect() as plugin:
        log.info("registered as %s, role %s", plugin.identity.instance_id, plugin.identity.role)
        log.info(
            "may publish %s; may subscribe %s",
            ", ".join(plugin.grants.publish) or "nothing",
            ", ".join(plugin.grants.subscribe) or "nothing",
        )
        await plugin.report(healthy=True, detail="started")
        await stopped.wait()
        log.info("stopping")


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    asyncio.run(run())


if __name__ == "__main__":
    main()
