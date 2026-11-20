"""An example, not wired in: a vendor's WebSocket stream, the plugin's own,
kept open by the SDK's `meridian.helpers.sessions.Supervisor`.

The reference vendor does not stream, so nothing runs this. A vendor that
does is read here: the socket and the vendor's messages are the plugin's
(rulings 1027 and 1028: Open Meridian ships no WebSocket client), from an
established library -- `websockets` below, a dependency of this plugin
only, which pyproject.toml shows commented until it is used. The SDK's
part is the supervision: reopening after a drop or silence, with jittered
backoff reset once steady, subscribing to what is wanted, and the
catch-up after a reopen.

    supervisor = stream.supervised(converter, url="wss://...", key=config.api_key)
    supervisor.want(["BTC-USD"])          # what the lake's standing wants name
    # in the data role class's run(): await supervisor.run()
"""

from __future__ import annotations

import json
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from meridian.helpers.sessions import Backoff, Supervisor

#: How long the vendor may say nothing before the session is taken for broken.
SILENT_SECONDS = 30.0


class Refused(Exception):
    """The vendor refused the key: waited out longer before trying again."""


async def websocket(url: str) -> Any:
    """A socket from the `websockets` library: add "websockets>=13,<16" to
    pyproject.toml's dependencies to use it."""
    import websockets  # type: ignore[import-not-found]  # the plugin's own dependency

    return await websockets.connect(url, open_timeout=15, max_size=16 * 1024 * 1024)


class VendorLink:
    """One session with the vendor's stream: a `helpers.sessions.Link`. Its
    messages here are JSON objects, `{"symbol": ..., "price": ...}`, handed
    on as they are for the conversion to read."""

    name = "vendor stream"

    def __init__(
        self, url: str, key: str, connect: Callable[[str], Awaitable[Any]] = websocket
    ) -> None:
        self.url, self.key, self.connect = url, key, connect
        self.socket: Any = None
        self.subscribed: set[str] = set()

    async def open(self) -> None:
        self.socket = await self.connect(self.url)
        self.subscribed = set()
        await self.socket.send(json.dumps({"action": "auth", "key": self.key}))
        answer = json.loads(await self.socket.recv())
        if answer.get("status") != "authenticated":
            raise Refused(str(answer.get("message") or "refused"))

    async def subscribe(self, wanted: Sequence[str]) -> None:
        now = set(wanted)
        if gone := sorted(self.subscribed - now):
            await self.socket.send(json.dumps({"action": "unsubscribe", "symbols": gone}))
        if added := sorted(now - self.subscribed):
            await self.socket.send(json.dumps({"action": "subscribe", "symbols": added}))
        self.subscribed = now

    async def receive(self) -> list[Any]:
        said = json.loads(await self.socket.recv())
        return [said] if said.get("symbol") else []  # a heartbeat says none

    async def close(self) -> None:
        if self.socket is not None:
            await self.socket.close()
            self.socket = None


def supervised(
    heard: Callable[[list[Any]], Awaitable[None]],
    caught_up: Callable[[dict[str, int]], Awaitable[int]],
    *,
    url: str,
    key: str,
    connect: Callable[[str], Awaitable[Any]] = websocket,
) -> Supervisor:
    """The vendor's stream kept open: `heard` given each message's
    statements, `caught_up` the gap after each reopen (when each symbol was
    last heard, ns) to read back from the vendor's history. No key, no
    session: said on the Connection page as "no key"."""
    return Supervisor(
        lambda: VendorLink(url, key, connect) if key else None,
        heard=heard,
        caught_up=caught_up,
        session="stream",
        silent_seconds=SILENT_SECONDS,
        refused=(Refused,),
        backoff=Backoff(first_s=1, most_s=60, jitter=0.2, steady_s=60),
        subject_of=lambda said: said.get("symbol"),
    )
