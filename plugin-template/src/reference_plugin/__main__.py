"""A Meridian plugin, as `meridian plugin new` writes one.

It runs on the sidecar it is launched beside, declaring its pages, says who
it was launched as and what the deployment lets it do, serves the pages
people reach through the dashboard (page.py), reports itself healthy, and
runs until it is stopped. Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential,
knows no other address, and cannot choose its own identity or grants: those
come from how the deployment launched it. Start from here -- each role the
plugin holds is one class, `meridian.roles.<role>.Role`, whose acts are the
steps that role may take, and nothing else reaches the bus -- and keep it
that way. This one declares no role (`[tool.meridian] roles = []`), so it
registers no class: page.py holds a custody class, `CUSTODY`, to show how
one is written, and its pages say there is nothing to link until it is
registered.
"""

import logging

import meridian

from .declaration import DECLARATION, PORT
from .page import CUSTODY, pages  # noqa: F401 - CUSTODY, once `custody` is declared


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    # Where the pages listen, on loopback beside the sidecar, which forwards
    # people's requests to them. Each page is declared with the levels it
    # serves, and the dashboard shows it under those buttons.
    #
    # A class is registered for each role the plugin declares, and only
    # those: `meridian plugin check` fails a declared role with no class, and
    # warns of a class for a role not declared. This plugin declares none.
    # Declaring `custody` in pyproject.toml's [tool.meridian] roles, register
    # page.py's class with it:
    #
    #     meridian.run(declaration=DECLARATION, roles=[CUSTODY], pages=pages, port=PORT)
    meridian.run(declaration=DECLARATION, roles=[], pages=pages, port=PORT)


if __name__ == "__main__":
    main()
