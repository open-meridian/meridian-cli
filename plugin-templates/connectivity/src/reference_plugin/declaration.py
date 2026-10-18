"""What this plugin declares beside its roles (W8.1): its settings -- here
only sample data, since the stand-in venue needs no key -- the storage its
trading notes and raw exchanges are kept in (a kind, `trading`, seven years
by default, so a restart sends nothing twice), its pages and the tools their
typed routes make, and its documentation, the Markdown in
`docs/` (`/plugin/docs` in its image), which a deployment admin reads on its
Documentation tab before launching it.

`connect(declaration=DECLARATION)` registers it, and `meridian plugin upload`
reads the same one from the built image, with the texts beside it. A version
registers exactly the declaration it was uploaded with: the port is fixed
here, never read from the environment, and this module imports every module
declaring a page or a route.
"""

from __future__ import annotations

import meridian
from meridian import RecordKind, Storage

from .page import TITLE, pages

#: Each exchange that places, changes or cancels: seven years by default.
TRADING = RecordKind("trading", "Trading", window_days=2555)

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000


@meridian.settings
class Config:
    sample_data: bool = meridian.sample_data(
        description="Place at the stand-in venue in memory. Development deployments only."
    )


DECLARATION = meridian.Declaration(
    settings=Config,
    storage=Storage(kinds=(TRADING,)),
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    documentation="docs",
)
