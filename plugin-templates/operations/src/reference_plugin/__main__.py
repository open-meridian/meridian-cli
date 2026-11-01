"""A Meridian operations plugin, as `meridian plugin new --role operations`
writes one.

It runs its operations role class on the sidecar it is launched beside,
declaring its settings and its pages: the class hears each activity the
custodians record, which the operations suite requires of every operations
plugin, and the pages act through it. `meridian.run` follows its settings,
reports the operations role's figures, serves its pages and runs until it is
stopped. Heartbeats are sent for it.
"""

import logging
from typing import Any

import meridian
from meridian.roles import operations

from .declaration import DECLARATION, PORT, Config
from .page import pages, running

log = logging.getLogger("reference_plugin")


class Book(operations.Role):
    """The operations role's class: its acts on `self`, made by the pages,
    and what the role hears."""

    async def on_activity_recorded(self, heard: meridian.Heard[Any]) -> None:
        # Each activity a custodian recorded, heard: a real plugin reconciles
        # it against the book here.
        log.info("activity %s recorded", heard.message.activity_id)


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    book = Book()
    running["role"] = book

    def configure(config: Config) -> None:
        running["due_business_days"] = config.due_business_days

    def figures() -> list[meridian.Figure]:
        return operations.figures(
            open_breaks=0, overdue_breaks=0, last_reconciliation="Not yet"
        )

    meridian.run(
        declaration=DECLARATION,
        roles=[book],
        pages=pages,
        port=PORT,
        on_settings=configure,
        figures=figures,
    )


if __name__ == "__main__":
    main()
