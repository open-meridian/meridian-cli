"""An example, not wired in: the vendor's rate limit, kept by the plugin.

How fast a vendor may be called, and what its 429 says, are the vendor's
terms (ruling 1028: vendor-facing code stays in the plugin), so this is the
plugin's own code, shaped to its vendor: a token bucket a second and a
minute, and a request sent again after a 429 once its `Retry-After` has
passed. The reference vendor reaches nothing, so nothing runs this; a
client calling a real API runs each call through `paced`:

    pace = Pace(per_second=5, per_minute=100)
    status, headers, body = paced(send, request, pace)     # in a thread
"""

from __future__ import annotations

import time
from collections.abc import Callable, Mapping
from typing import Any


class Pace:
    """At most `per_second` calls a second and `per_minute` a minute, each
    bucket holding `burst` calls."""

    def __init__(
        self,
        per_second: float = 0,
        per_minute: float = 0,
        burst: int = 1,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        self.rates = [r for r in (per_second, per_minute / 60) if r > 0]
        self.burst, self.clock = burst, clock
        self.tokens = [float(burst) for _ in self.rates]
        self.at = clock()

    def wait(self) -> float:
        """Count a call: how many seconds from now it may go."""
        now = self.clock()
        elapsed, self.at = now - self.at, now
        wait = 0.0
        for i, rate in enumerate(self.rates):
            self.tokens[i] = min(self.burst, self.tokens[i] + elapsed * rate) - 1
            wait = max(wait, -self.tokens[i] / rate)
        return wait


def retry_after(headers: Mapping[str, str], otherwise: float) -> float:
    """The seconds a 429 says to wait (`Retry-After`, as a delay), else
    `otherwise`."""
    said = next((v for k, v in headers.items() if k.lower() == "retry-after"), "")
    return float(said) if str(said).strip().isdigit() else otherwise


def paced(
    send: Callable[[Any], tuple[int, Mapping[str, str], bytes]],
    request: Any,
    pace: Pace,
    retries: int = 3,
    sleep: Callable[[float], Any] = time.sleep,
) -> tuple[int, Mapping[str, str], bytes]:
    """`send(request)` within the pace, sent again after a 429 when its
    `Retry-After` has passed (1, 2, 4 s otherwise), at most `retries` times."""
    for attempt in range(retries + 1):
        sleep(pace.wait())
        status, headers, body = send(request)
        if status != 429 or attempt == retries:
            break
        sleep(retry_after(headers, 2.0**attempt))
    return status, headers, body
