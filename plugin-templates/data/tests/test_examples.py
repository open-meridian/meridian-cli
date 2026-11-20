"""The two examples (stream.py, pacing.py), against fakes: replace them as
you replace the examples."""

from __future__ import annotations

import asyncio
import json
from typing import Any

from meridian.helpers.sessions import Backoff

from reference_plugin import pacing, stream


class Socket:
    """A vendor's socket saying `said`, then dropping."""

    def __init__(self, said: list[dict[str, Any]]) -> None:
        self.said = [json.dumps({"status": "authenticated"}), *map(json.dumps, said)]
        self.sent: list[dict[str, Any]] = []

    async def send(self, text: str) -> None:
        self.sent.append(json.loads(text))

    async def recv(self) -> str:
        if not self.said:
            raise ConnectionError("dropped")
        return self.said.pop(0)

    async def close(self) -> None:
        return None


def test_the_stream_is_subscribed_heard_and_caught_up_after_a_reopen() -> None:
    sockets = [Socket([{"symbol": "BTC-USD", "price": "62431.27"}]), Socket([])]
    heard: list[Any] = []
    gaps: list[dict[str, int]] = []

    async def connect(url: str) -> Socket:
        return sockets.pop(0)

    async def hear(stated: list[Any]) -> None:
        heard.extend(stated)

    async def caught_up(since: dict[str, int]) -> int:
        gaps.append(since)
        return 0

    async def run() -> None:
        supervisor = stream.supervised(hear, caught_up, url="wss://x", key="k", connect=connect)
        supervisor.backoff = Backoff(first_s=0.001, most_s=0.001, jitter=0, steady_s=60)
        supervisor.want(["BTC-USD"])
        await supervisor.run(once=True)
        await supervisor.run(once=True)
        assert supervisor.state.opened == 2

    asyncio.run(run())
    assert [s["symbol"] for s in heard] == ["BTC-USD"]
    assert list(gaps[0]) == ["BTC-USD"]


def test_no_key_opens_no_session() -> None:
    async def hear(stated: list[Any]) -> None:
        return None

    async def caught_up(since: dict[str, int]) -> int:
        return 0

    supervisor = stream.supervised(hear, caught_up, url="wss://x", key="")
    supervisor.want(["BTC-USD"])
    asyncio.run(supervisor.run(once=True))
    assert supervisor.state.state == "no key"


def test_calls_are_paced_and_a_429_waited_out_as_it_says() -> None:
    now = [0.0]
    pace = pacing.Pace(per_second=1, burst=2, clock=lambda: now[0])
    assert [pace.wait(), pace.wait(), pace.wait()] == [0.0, 0.0, 1.0]
    answers = [(429, {"Retry-After": "3"}, b""), (200, {}, b"ok")]
    slept: list[float] = []
    status, _, body = pacing.paced(
        lambda request: answers.pop(0),
        "GET /daily",
        pacing.Pace(per_minute=6000),
        sleep=slept.append,
    )
    assert (status, body) == (200, b"ok") and 3.0 in slept
