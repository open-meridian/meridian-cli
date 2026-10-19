"""What this plugin declares beside its role (W8.1): its page and the tool
its typed route makes, and its documentation, the Markdown in `docs/`
(`/plugin/docs` in its image), which a deployment admin reads on its
Documentation tab before launching it.

`connect(declaration=DECLARATION)` registers it, and `meridian plugin upload`
reads the same one from the built image, with the texts beside it. A version
registers exactly the declaration it was uploaded with: the port is fixed
here, never read from the environment, and this module imports every module
declaring a page or a route.

It declares no storage: `pricing` is not a role at the edge
(decisions/028), so it keeps the last batch it answered in memory.
"""

from __future__ import annotations

import meridian

from .page import TITLE, pages

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000

DECLARATION = meridian.Declaration(
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    documentation="docs",
)
