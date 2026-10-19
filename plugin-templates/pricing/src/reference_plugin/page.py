"""The plugin's page, as people see it through the deployment's dashboard.

- Batches (/), at `admin`: the model ids it implements, the batches it has
  answered since it started, the last sequence answered, and each batch left
  unanswered with why; counts and codes only, never a price. It declares the
  typed data it renders (`answers=`), so it is also a read tool on the
  deployment's MCP surface, `read_batches`.

Replace it with your plugin's own pages; keep declaring them this way.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import meridian

TITLE = "Reference pricing"

pages = meridian.Pages(TITLE, templates=Path(__file__).parent / "templates")

#: What __main__.py gives the page: the running Pricer.
running: dict[str, Any] = {"pricer": None}


@dataclass(frozen=True)
class Unanswered:
    batch_id: str
    why: str


@dataclass(frozen=True)
class BatchesRead:
    """What Batches shows: the model ids implemented, the batches answered
    and the last sequence, and each left unanswered with why."""

    models: list[str] = field(default_factory=list)
    answered: int = 0
    sequence: int = 0
    unanswered: list[Unanswered] = field(default_factory=list)


def batches_read() -> BatchesRead:
    pricer = running["pricer"]
    if pricer is None:
        return BatchesRead()
    return BatchesRead(
        models=sorted(pricer.models),
        answered=pricer.answered,
        sequence=pricer.sequence.value,
        unanswered=[Unanswered(b, why) for b, why in sorted(pricer.unanswered.items())],
    )


@pages.page("/", "Batches", levels="admin", answers=BatchesRead, name="read_batches")
async def batches(request: meridian.Request) -> meridian.Response:
    """The batches this plugin has answered, and any left unanswered."""
    return pages.answer("batches.html", batches_read())
