"""What this plugin declares beside its roles (W8.1): its catalogue.

A `dgm` declares each dataset it serves the lake, from code: its key, its
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

DECLARATION = meridian.Declaration(catalogue=[DAILY])
