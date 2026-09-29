"""The page, and the operation it sends, against a stand-in for the sidecar.

`meridian plugin check --run-tests` runs these, and so does the plugin's CI.
None needs a deployment. `Sidecar` is the SDK's own operations with the
transport replaced: a call goes through the SDK's real conversion and is kept
as the message the sidecar would have received. `caller_header` is the
`Meridian-Caller` header a sidecar forwards with each request for the page.

Replace these as you replace the page: at least each page rendered for a
caller, and each operation the plugin sends.
"""

from __future__ import annotations

import asyncio
import base64
import threading
import urllib.error
import urllib.request
from collections.abc import Iterable, Iterator
from contextlib import contextmanager
from typing import Any, cast

import meridian
from meridian.operations import Operations
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.v1 import sidecar_pb2

from reference_plugin.page import render, serve


class _Named:
    """Each operation, named for itself, so what was sent says which it was."""

    def __getattr__(self, name: str) -> str:
        return name


class Sidecar(Operations):
    """The SDK's operations, answered here rather than by a sidecar."""

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


def caller_header(read: Iterable[str] = (), write: Iterable[str] = ()) -> str:
    """The header a sidecar forwards for Ada: the accounts she may read, and
    those she may write, through this plugin."""
    claims = sidecar_pb2.CallerClaims(
        subject="local|ada",
        display_name="Ada Park",
        read_account_ids=list(read),
        write_account_ids=list(write),
    )
    assertion = sidecar_pb2.CallerAssertion(claims=claims.SerializeToString())
    return base64.urlsafe_b64encode(assertion.SerializeToString()).decode().rstrip("=")


@contextmanager
def served(sidecar: Sidecar) -> Iterator[str]:
    """The page on a free loopback port, its operations run on a loop of its
    own as the plugin's are; yields its address."""
    loop = asyncio.new_event_loop()
    running = threading.Thread(target=loop.run_forever, daemon=True)
    running.start()
    server = serve(cast(meridian.Plugin, sidecar), loop, 0)
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        loop.call_soon_threadsafe(loop.stop)
        running.join()
        loop.close()


def ask(address: str, path: str, header: str | None, method: str = "GET") -> tuple[int, str]:
    request = urllib.request.Request(
        address + path, method=method, data=b"" if method == "POST" else None
    )
    if header is not None:
        request.add_header("Meridian-Caller", header)
    try:
        with urllib.request.urlopen(request, timeout=10) as answer:
            return answer.status, answer.read().decode()
    except urllib.error.HTTPError as answer:
        return answer.code, answer.read().decode()


def test_the_page_shows_the_accounts_the_caller_may_read_and_write() -> None:
    caller = meridian.Caller.from_header(
        caller_header(read={"ACC-1", "ACC-2"}, write={"ACC-2"})
    )

    page = render(caller)

    assert "Ada Park" in page
    assert "<td><code>ACC-1</code></td><td>read</td>" in page
    assert "<td><code>ACC-2</code></td><td>read and write</td>" in page


def test_opening_a_statement_is_sent_for_the_person_asking() -> None:
    sidecar = Sidecar()

    with served(sidecar) as address:
        status, page = ask(
            address, "/statement", caller_header(read={"A"}, write={"A"}), "POST"
        )

    assert status == 200
    assert "Opened statement STMT-1 for you." in page
    [(operation, params)] = sidecar.sent
    assert operation == "RecordHoldingsStatement"
    # Sent for Ada, so the sidecar decides whether she may write through the plugin.
    assert sidecar_pb2.CallerClaims.FromString(params.acting_for.claims).subject == "local|ada"


def test_what_the_sidecar_refuses_is_said_on_the_page() -> None:
    sidecar = Sidecar(
        refuse=meridian.NotGranted("RecordHoldingsStatement", "not hers to write")
    )

    with served(sidecar) as address:
        status, page = ask(address, "/statement", caller_header(read={"A"}), "POST")

    assert status == 200
    assert '<div class="notice bad" role="status">Refused:' in page
    assert "not hers to write" in page


def test_a_request_the_sidecar_did_not_forward_is_turned_away() -> None:
    sidecar = Sidecar()

    with served(sidecar) as address:
        status, _ = ask(address, "/", header=None)

    assert status == 401
    assert sidecar.sent == []
