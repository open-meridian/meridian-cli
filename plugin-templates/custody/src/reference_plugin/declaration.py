"""What this plugin declares beside its roles (W8.1): its settings, in the
custody role's order (spec/plugin-pages-follow-their-role): the credentials,
each a required secret in the custodian's own words; "Read every" and "Stale
after", where the custodian is read on a schedule; the custodian's own; then
sample data.

From contract v25 it is the whole of what the version says about itself:
its pages and the tools their typed routes make, that it reads external
accounts, and its documentation, the Markdown in `docs/` (relative to where
it runs: `/plugin/docs` in its image), which a deployment admin reads on its
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
from meridian.roles import custody

from .page import TITLE, pages

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000


@meridian.settings
class Config:
    api_key: str = meridian.setting(
        "API key",
        required=True,
        secret=True,
        description="The key the custodian issued this deployment, in its developer settings.",
    )
    poll_seconds: int = custody.poll_seconds()
    stale_after_hours: int = custody.stale_after_hours()
    sample_data: bool = meridian.sample_data()


DECLARATION = meridian.Declaration(
    settings=Config,
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    reads_external_accounts=True,
    documentation="docs",
)
