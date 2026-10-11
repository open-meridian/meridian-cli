"""What this plugin declares beside its roles (W8.1): its settings, in the
custody role's order (spec/plugin-pages-follow-their-role): the credentials,
each a required secret in the custodian's own words; "Read every" and "Stale
after", where the custodian is read on a schedule; the custodian's own; then
sample data.

`connect(declaration=DECLARATION)` sends it at registration, and `meridian
plugin upload` reads the same one from the built image, as pyproject.toml's
`[tool.meridian]` names it.
"""

from __future__ import annotations

import meridian
from meridian.roles import custody


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


DECLARATION = meridian.Declaration(settings=Config)
