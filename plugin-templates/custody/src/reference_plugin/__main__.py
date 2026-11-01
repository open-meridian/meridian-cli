"""A Meridian custody plugin, as `meridian plugin new --role custody` writes one.

It runs its custody role class (convert.py) on the sidecar it is launched
beside, declaring its settings and its pages: the class follows the account
scope and its links for the pages, and `meridian.run` reports the custody
role's figures, serves its pages and runs until it is stopped. Heartbeats
are sent for it.
"""

import logging

import meridian
from meridian.roles import custody

from .convert import Converter
from .custodian import Custodian
from .declaration import DECLARATION, PORT, Config
from .page import pages, use


class Custody(Converter):
    """The custody role's class as it runs: its conversion, and the account
    scope with its links followed for the pages."""

    def __init__(self, custodian: Custodian) -> None:
        self.custodian = custodian
        self.key_set = False

    async def run(self) -> None:
        # The account scope and this plugin's links, on start and on every
        # change: the pages hold them from here, never the plugin's own.
        async for scope in self.plugin.account_scope():
            use(self.custodian, key_set=self.key_set, scope=scope, role=self)


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    custodian = Custodian()
    held = Custody(custodian)
    use(custodian, key_set=False, role=held)

    def configure(config: Config) -> None:
        held.key_set = bool(config.api_key)
        use(custodian, key_set=held.key_set, role=held)

    def figures() -> list[meridian.Figure]:
        return custody.figures(
            connections=1, accounts_reached=len(custodian.accounts), last_read="Not yet"
        )

    meridian.run(
        declaration=DECLARATION,
        roles=[held],
        pages=pages,
        port=PORT,
        on_settings=configure,
        figures=figures,
    )


if __name__ == "__main__":
    main()
