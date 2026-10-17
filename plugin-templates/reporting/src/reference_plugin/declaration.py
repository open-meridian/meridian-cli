"""What this plugin declares about itself (W8.1, contract v25): the reporting
role's common setting, its pages and the tools their typed routes make, and
its documentation, the Markdown in `docs/` (relative to where it runs:
`/plugin/docs` in its image), which a deployment admin reads on its
Documentation tab before launching it.

`connect(declaration=DECLARATION)` registers it, and `meridian plugin upload`
reads the same one from the built image, as pyproject.toml's
`[tool.meridian]` names it, with the texts beside it. A version registers
exactly the declaration it was uploaded with: the port is fixed here, never
read from the environment, and this module imports every module declaring a
page or a route.
"""

from __future__ import annotations

import meridian
from meridian.roles import reporting

from .page import TITLE, pages

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000

DECLARATION = meridian.Declaration(
    # The reporting role's common setting (spec/plugin-pages-follow-their-
    # role): every reporting plugin values in a currency.
    settings=[reporting.reporting_currency()],
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    documentation="docs",
)
