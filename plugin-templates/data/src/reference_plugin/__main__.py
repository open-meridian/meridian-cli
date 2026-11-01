"""A Meridian data plugin, as `meridian plugin new --role data` writes one.

It runs its data role class (convert.py) on the sidecar it is launched
beside, declaring its settings, its catalogue and its pages: the class reads
which of its datasets a deployment admin has licensed and resolves the
symbols its vendor serves it, then hears what the lake wants of its datasets
and records against each want from its vendor, or declines what the vendor
does not serve. `meridian.run` reports it healthy once it has started,
reports the data role's figures, serves its pages and runs until it is
stopped. Heartbeats are sent for it.

Everything a plugin does goes through that sidecar. It holds no credential
of the deployment's, knows no other address of it, and cannot choose its own
identity or grants: those come from how the deployment launched it. Its
vendor is its own to reach (vendor.py stands in for it here), with a key in a
secret setting where the vendor needs one.
"""

import logging

import meridian
from meridian.roles import data

from .convert import Converter
from .declaration import DECLARATION, PORT, Config
from .page import pages, use


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    converter = Converter()
    use(converter)

    def configure(config: Config) -> None:
        converter.sample_data = config.sample_data

    def figures() -> list[meridian.Figure]:
        return data.figures(
            connection="Sample data" if converter.sample_data else "Answering",
            entitled_at_vendor=f"{len(DECLARATION.catalogue)} of "
            f"{len(DECLARATION.catalogue)} datasets",
            stream="Does not stream",
            rate_use=f"{converter.vendor.calls} of "
            f"{converter.vendor.limit_per_minute} a minute",
        )

    meridian.run(
        declaration=DECLARATION,
        roles=[converter],
        pages=pages,
        port=PORT,
        on_settings=configure,
        figures=figures,
    )


if __name__ == "__main__":
    main()
