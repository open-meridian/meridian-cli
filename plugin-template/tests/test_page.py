"""The pages, and the operation they send, against a stand-in for the sidecar.

`meridian plugin check --run-tests` runs these, and so does the plugin's CI.
None needs a deployment. `Sidecar` is the SDK's own operations with the
transport replaced: a call goes through the SDK's real conversion and is kept
as the message the sidecar would have received. `PageClient` asks the pages
as the sidecar forwards a request, for a session opened by Manage (`admin`),
Open (`write`) or View (`read`), with the person's accounts cut to it.

Replace these as you replace the pages: at least each page rendered at each
level, no account data under Manage, and each operation the plugin sends.
"""

from __future__ import annotations

import asyncio
import threading
import urllib.error
import urllib.request
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any, cast

import meridian
from meridian.operations import Operations
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.testing import PageClient, caller_header
from meridian.v1 import sidecar_pb2

from reference_plugin.page import pages


class _Named:
    """Each operation, named for itself, so what was sent says which it was."""

    def __getattr__(self, name: str) -> str:
        return name


class Sidecar(Operations):
    """The SDK's operations, answered here rather than by a sidecar, and who
    the plugin was launched as."""

    identity = meridian.Identity("reference-1", roles=("custody",))
    grants = meridian.Grants(publish=("platform.street.command.record-holdings-statement",))

    def __init__(self, refuse: Exception | None = None) -> None:
        self.sent: list[tuple[str, Any]] = []
        self._refuse = refuse

    def _operations(self) -> Any:
        return _Named()

    async def _operate(self, method: Any, params: Any) -> Any:
        if self._refuse is not None:
            raise self._refuse
        self.sent.append((cast(str, method), params))
        if method == "RecordHoldingsStatement":
            return ops.RecordHoldingsStatementResult(statement_id=f"STMT-{len(self.sent)}")
        raise AssertionError(f"the page sent {method}, which no test here expects")


def client(sidecar: Sidecar | None = None) -> PageClient:
    """Ada, who may read ACC-1 and ACC-2 and write ACC-2 through the plugin."""
    return PageClient(pages, sidecar or Sidecar(), read={"ACC-1", "ACC-2"}, write={"ACC-2"})


def test_each_page_is_served_at_its_levels_and_refused_at_the_others() -> None:
    served = {(r.page.path, r.level): r.response.status for r in client().every_page()}
    assert served == {
        ("/setup", "admin"): 200,
        ("/setup", "write"): 403,
        ("/setup", "read"): 403,
        ("/", "admin"): 403,
        ("/", "write"): 200,
        ("/", "read"): 200,
    }


def test_manage_shows_the_plugins_setup_and_no_accounts_data() -> None:
    page = client().get("/setup", "admin").text
    assert "Ada Park" in page and "<code>reference-1</code>" in page and "custody" in page
    # What the plugin holds for an account -- here, the statement it opened
    # for one -- is never on a page at admin. An account's identity may be: a
    # Manage page may list accounts by name, to link to.
    sidecar = Sidecar()
    assert "STMT-1" in client(sidecar).post("/statement", "write").text
    client(sidecar).assert_no_account_data("STMT-1")


def test_the_accounts_page_shows_what_the_caller_may_read_and_write() -> None:
    page = client().get("/", "write").text
    assert "<td><code>ACC-1</code></td><td>read</td>" in page
    assert "<td><code>ACC-2</code></td><td>read and write</td>" in page
    assert 'action="/statement"' in page
    # View shows the same accounts, read-only, and no action.
    viewed = client().get("/", "read").text
    assert "<td><code>ACC-2</code></td><td>read</td>" in viewed
    assert 'action="/statement"' not in viewed


def test_opening_a_statement_is_sent_for_the_person_asking() -> None:
    sidecar = Sidecar()

    page = client(sidecar).post("/statement", "write").text

    assert "Opened statement STMT-1 for you." in page
    [(operation, params)] = sidecar.sent
    assert operation == "RecordHoldingsStatement"
    # Sent for Ada, so the sidecar decides whether she may write through the plugin.
    assert sidecar_pb2.CallerClaims.FromString(params.acting_for.claims).subject == "local|ada"


def test_only_a_session_opened_by_open_may_write() -> None:
    sidecar = Sidecar()
    for level in ("read", "admin"):
        assert client(sidecar).post("/statement", level).status == 403
    assert sidecar.sent == []


def test_a_post_without_this_pages_token_is_refused_before_anything_is_sent() -> None:
    # The plugin's host keeps the person's session in a cookie, so another
    # page could post here as them; only this plugin's own page has the token.
    sidecar = Sidecar()
    ben = PageClient(pages, sidecar, subject="local|ben").caller("write")
    for form in ({}, {"csrf": "0" * 64}, {"csrf": pages.csrf_token(ben)}):
        answered = client(sidecar).request("POST", "/statement", "write", form=form)
        assert answered.status == 403 and "Reload the page" in answered.text
    assert sidecar.sent == []
    # The page's own form carries it.
    page = client(sidecar).get("/", "write").text
    token = pages.csrf_token(client().caller("write"))
    assert f'<input type="hidden" name="csrf" value="{token}">' in page


def test_what_the_sidecar_refuses_is_said_on_the_page() -> None:
    sidecar = Sidecar(
        refuse=meridian.NotGranted("RecordHoldingsStatement", "not hers to write")
    )

    page = client(sidecar).post("/statement", "write").text

    assert '<div class="notice bad" role="status">Refused:' in page
    assert "not hers to write" in page


@contextmanager
def served(sidecar: Sidecar) -> Iterator[str]:
    """The pages on a free loopback port, their views run on a loop of their
    own as the plugin's are; yields the address."""
    loop = asyncio.new_event_loop()
    running = threading.Thread(target=loop.run_forever, daemon=True)
    running.start()
    server = pages.serve(cast(meridian.Plugin, sidecar), 0, loop=loop)
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        loop.call_soon_threadsafe(loop.stop)
        running.join()
        loop.close()


def ask(address: str, path: str, header: str | None) -> int:
    request = urllib.request.Request(address + path)
    if header is not None:
        request.add_header("Meridian-Caller", header)
    try:
        with urllib.request.urlopen(request, timeout=10) as answer:
            return int(answer.status)
    except urllib.error.HTTPError as answer:
        return answer.code


def test_a_request_the_sidecar_did_not_forward_is_turned_away() -> None:
    with served(Sidecar()) as address:
        assert ask(address, "/", header=None) == 401
        assert ask(address, "/", caller_header("read")) == 200
