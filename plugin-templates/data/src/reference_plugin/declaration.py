"""What this plugin declares beside its roles (W8.1): its settings and its
catalogue.

A data plugin declares each dataset it serves the lake, from code: its key, its
vendor, the data types it fills by their dictionary entries, how its rows can
arrive, how often its source updates and how far back it reaches, the terms
the vendor's standard terms impose, and the day a daily value's business date
is in. A deployment admin licenses each dataset, and entitles the plugins
that may read it, on the dashboard's Data sources page; until then nothing
is served from it.

From contract v25 it is the whole of what the version says about itself:
its pages and the tools their typed routes make, and its documentation, the
Markdown in `docs/` (relative to where it runs: `/plugin/docs` in its
image), which a deployment admin reads on its Documentation tab before
launching it.

`meridian.run(declaration=DECLARATION, ...)` registers it, and `meridian plugin upload`
reads the same one from the built image, as pyproject.toml's
`[tool.meridian]` names it, with the texts beside it. A version registers
exactly the declaration it was uploaded with: the port is fixed here, never
read from the environment, and this module imports every module declaring a
page or a route.
"""

from __future__ import annotations

import meridian

from .page import TITLE, pages
from .vendor import VENDOR

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000

#: The vendor's daily closes and bars: fetched when the lake wants them
#: (`pull`), the day ending at midnight UTC, five years back. Not one venue's,
#: so it names none; each row names the venue its price is from, where the
#: vendor says.
DAILY = meridian.DatasetDeclaration(
    key="daily",
    vendor=VENDOR,
    data_types=["meridian.v1.Price", "meridian.v1.Bar"],
    modes=["pull"],
    cadence=86_400,
    history=1_826,
    day_time_zone="Etc/UTC",
    # The deployment admin settles the terms with the vendor and sets the
    # licence on Data sources; the plugin only describes them, in plain words
    # (contract 30, ruling 1032), shown on Data sources and in its docs.
    terms_and_pricing=(
        "Daily closes and bars, five years back. Included in the vendor's free plan "
        "for personal use; business use needs its Business plan ($99 a month). Not "
        "for redistribution; the vendor asks that pages showing it say 'Data from "
        "Reference vendor'."
    ),
    # A gap the lake's write-ahead stream leaves is asked again of a vendor
    # serving history (W10.14).
    gap_recovery="backfill",
)


@meridian.settings
class Config:
    """Its settings, in the data role's order (spec/plugin-pages-follow-their-
    role): `meridian.environment()` first where the vendor has more than one,
    then each credential, a required secret in the vendor's words and gated
    on the environment where they differ by it, then the vendor's own, then
    sample data. The stand-in vendor needs no key and has one environment,
    so only sample data is declared; a real vendor's key goes here, as
    `api_key: str = meridian.setting("API key", required=True, secret=True)`.
    """

    sample_data: bool = meridian.sample_data()


DECLARATION = meridian.Declaration(
    settings=Config,
    catalogue=[DAILY],
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    documentation="docs",
)
