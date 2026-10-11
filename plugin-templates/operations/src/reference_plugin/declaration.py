"""What this plugin declares beside its roles (W8.1): its settings, the
operations role's common ones (spec/plugin-pages-follow-their-role).
"Reconcile" is off until an admin turns it on, since it acts with no person
behind it. It reads only its deployment, so it declares no sample data.

`connect(declaration=DECLARATION)` sends it at registration, and `meridian
plugin upload` reads the same one from the built image.
"""

from __future__ import annotations

import meridian
from meridian.roles import operations


@meridian.settings
class Config:
    reconcile: bool = operations.reconcile()
    due_business_days: int = operations.due_business_days()
    street_fresh_within_hours: int = operations.street_fresh_within_hours()


DECLARATION = meridian.Declaration(settings=Config)
