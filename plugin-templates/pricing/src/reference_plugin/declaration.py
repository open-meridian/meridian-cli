"""What this plugin declares beside its role (W8.1): the storage the last
batch it answered is kept in (a kind, `pricing`, so a restart reads the
batches still waiting after it), its page and the tool its typed route
makes, and its documentation, the Markdown in `docs/` (`/plugin/docs` in its
image), which a deployment admin reads on its Documentation tab before
launching it.

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

#: The last batch sequence it answered: a year by default.
PRICING = RecordKind("pricing", "Pricing", window_days=365)

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000

DECLARATION = meridian.Declaration(
    storage=Storage(kinds=(PRICING,)),
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    documentation="docs",
)
