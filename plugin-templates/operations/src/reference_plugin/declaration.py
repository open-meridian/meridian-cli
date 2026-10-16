"""What this plugin declares beside its roles (W8.1): its settings, the
operations role's common ones (spec/plugin-pages-follow-their-role).
"Reconcile" is off until an admin turns it on, since it acts with no person
behind it. It reads only its deployment, so it declares no sample data.

From contract v25 it is the whole of what the version says about itself:
its pages and the tools their typed routes make, and its documentation, the
Markdown in `docs/` (relative to where it runs: `/plugin/docs` in its
image), which a deployment admin reads on its Documentation tab before
launching it. An operations plugin may also declare its affirmation
deadline, `affirmation_deadline=21 * 60`: minutes after midnight in the
deployment's time zone by which the firm allocates, confirms and affirms
its trades; none is declared here.

`connect(declaration=DECLARATION)` registers it, and `meridian plugin upload`
reads the same one from the built image, with the texts beside it. A
version registers exactly the declaration it was uploaded with: the port is
fixed here, never read from the environment, and this module imports every
module declaring a page or a route.
"""

from __future__ import annotations

import meridian
from meridian.roles import operations

from .page import TITLE, pages

#: The port its pages are served on, on loopback, where its sidecar forwards.
PORT = 8000


@meridian.settings
class Config:
    reconcile: bool = operations.reconcile()
    due_business_days: int = operations.due_business_days()
    street_fresh_within_hours: int = operations.street_fresh_within_hours()


DECLARATION = meridian.Declaration(
    settings=Config,
    interface=meridian.Interface(port=PORT, title=TITLE, pages=pages),
    documentation="docs",
)
