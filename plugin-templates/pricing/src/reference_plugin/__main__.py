"""A Meridian `pricing` plugin, as `meridian plugin new --role pricing` writes
one.

It runs the SDK's Pricer over its models (model.py) -- the pricing role's
class -- on the sidecar it is launched beside, declaring its page: on start
the batches still waiting after the last sequence it answered, then each
batch routed to it, answered whole, until it is stopped. The last sequence
answered is kept in memory: `pricing` is not a role at the edge and holds no
storage (decisions/028), so after a restart it reads every batch still
waiting, and a batch answered twice is harmless, named by its hash.
"""

import logging

import meridian
from meridian.roles import pricing

from .declaration import DECLARATION, PORT
from .model import MODELS
from .page import pages, running


def main() -> None:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
    )
    pricer = pricing.Pricer(MODELS, sequence=pricing.Sequence())
    running["pricer"] = pricer
    meridian.run(declaration=DECLARATION, roles=[pricer], pages=pages, port=PORT)


if __name__ == "__main__":
    main()
