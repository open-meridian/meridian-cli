"""A Meridian `reporting` plugin, as `meridian plugin new --role reporting`
writes one.

It runs its reporting role class (report.py) on the sidecar it is launched
beside, declaring its pages: the class reads the datasets it may read and
the positions in its account scope on start, then hears what the lake
records for the instruments held. Its pages read the book and the lake as
people open them, and record nothing. `meridian.run` reports it healthy and
runs it until it is stopped. Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential,
knows no other address, and cannot choose its own identity or grants: those
come from how the deployment launched it.
"""

import logging

import meridian
from meridian.roles import reporting

from .declaration import DECLARATION, PORT
from .page import pages, use
from .report import Report


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    report = Report()
    use(report)

    def configure(settings: meridian.Settings) -> None:
        report.reporting_currency = str(settings.values["reporting_currency"])

    meridian.run(
        declaration=DECLARATION,
        roles=[report],
        pages=pages,
        port=PORT,
        on_settings=configure,
        # The reporting role's figures, first on its Summary.
        figures=lambda: reporting.figures(last_valued="Not yet"),
    )


if __name__ == "__main__":
    main()
