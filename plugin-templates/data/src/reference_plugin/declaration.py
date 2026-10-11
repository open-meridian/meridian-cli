"""What this plugin declares beside its roles (W8.1): its settings and its
catalogue.

A data plugin declares each dataset it serves the lake, from code: its key, its
vendor, the data types it fills by their dictionary entries, how its rows can
arrive, how often its source updates and how far back it reaches, the terms
the vendor's standard terms impose, and the day a daily value's business date
is in. A deployment admin licenses each dataset, and entitles the plugins
that may read it, on the dashboard's Data sources page; until then nothing
is served from it.

`connect(declaration=DECLARATION)` sends it at registration, and `meridian
plugin upload` reads the same one from the built image, as pyproject.toml's
`[tool.meridian]` names it.
"""

from __future__ import annotations

import meridian

from .vendor import VENDOR

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
    licence_default=meridian.DatasetLicence(kept=True),
    day_time_zone="Etc/UTC",
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


DECLARATION = meridian.Declaration(settings=Config, catalogue=[DAILY])
