"""What this plugin declares about itself (W8.1, contract v25): its pages, the
tools their typed routes make, and its documentation, the Markdown in
`docs/`.

`connect(declaration=DECLARATION)` registers it, and `meridian plugin upload`
reads the same one from the built image, as pyproject.toml's
`[tool.meridian]` names it, with the texts of `docs/` beside it. A version
registers exactly the declaration it was uploaded with, so everything it says
about itself is here: the interface's port is fixed, never read from the
environment, and this module imports every module declaring a page or a
route. Settings, when it has some, go here too (`settings=Config`).

`documentation` names the folder relative to where the plugin runs: in its
image, `/plugin/docs`. A deployment admin reads it on the plugin's
Documentation tab before launching it.
"""

from __future__ import annotations

import meridian

from .page import TITLE, pages

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000

INTERFACE = meridian.Interface(port=PORT, title=TITLE, pages=pages)

DECLARATION = meridian.Declaration(interface=INTERFACE, documentation="docs")
