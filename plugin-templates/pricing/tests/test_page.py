"""The page, against a stand-in for the sidecar: Batches under Manage, and
its tool, with counts and codes only."""

from __future__ import annotations

import asyncio

from meridian.roles import pricing
from meridian.suites import Recorder
from meridian.testing import PageClient

from reference_plugin.model import MODELS
from reference_plugin.page import pages, running


def test_batches_is_served_under_manage_alone_with_its_tool() -> None:
    pricer = pricing.Pricer(Recorder("reference-1"), MODELS)
    asyncio.run(pricer.catch_up())
    running["pricer"] = pricer
    client = PageClient(pages)
    assert client.get("/", "admin").status == 200
    assert client.get("/", "read").status == 403
    answered = client.call_tool("read_batches", level="admin").data
    assert answered["models"] == ["quoted_price"]
    assert answered["answered"] == 0 and answered["unanswered"] == []


def test_it_declares_no_storage() -> None:
    """`pricing` is not a role at the edge (decisions/028): it keeps the last
    batch it answered in memory, and holds no storage."""
    from reference_plugin.declaration import DECLARATION

    assert DECLARATION.storage is None
