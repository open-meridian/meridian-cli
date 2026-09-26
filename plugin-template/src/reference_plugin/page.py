"""The plugin's page, as people see it through the deployment's dashboard.

Served on loopback, where only this plugin's sidecar reaches it: the sidecar
verified who is asking before forwarding the request, and says so in one
`Meridian-Caller` header, which `meridian.Caller` reads. Nothing here checks a
signature, holds a key or keeps a session -- that is the point.

It shows who is asking and what they may see through this plugin, tag by tag,
and offers one action that writes for them: opening an empty holdings
statement, sent with `acting_for` so the sidecar decides whether they may.
Replace it with your plugin's own pages; keep reading the caller the same way.

The standard library's server, so the plugin needs nothing its base image
does not already have. A framework on ASGI can use `meridian.CallerMiddleware`
instead.
"""

from __future__ import annotations

import asyncio
import html
import http.server
import threading
import time
import uuid
from typing import Any

import meridian

TITLE = "Reference plugin"


def render(caller: meridian.Caller, notice: str = "") -> str:
    rows = "".join(
        "<tr><td>{}</td><td>{}</td><td>{}</td></tr>".format(
            html.escape(held.tag),
            html.escape(", ".join(sorted(held.read)) or "none"),
            html.escape(", ".join(sorted(held.write)) or "none"),
        )
        for held in caller.access
    )
    said = f"<p><strong>{html.escape(notice)}</strong></p>" if notice else ""
    return (
        f'<!doctype html><html lang="en"><head><meta charset="utf-8">'
        f"<title>{TITLE}</title></head><body>"
        f"<h1>{TITLE}</h1>"
        f"<p>Signed in as <strong>{html.escape(caller.display_name)}</strong>.</p>"
        f"<h2>What you may see here</h2>"
        f"<table><tr><th>Tag</th><th>Read</th><th>Write</th></tr>{rows}</table>"
        f"{said}"
        f'<form method="post" action="/statement">'
        f"<button>Open an empty statement for me</button></form>"
        f"</body></html>"
    )


def serve(plugin: meridian.Plugin, loop: asyncio.AbstractEventLoop, port: int) -> Any:
    """Start the page on 127.0.0.1:`port`, in a thread; the returned server's
    `shutdown()` stops it. Operations run on `loop`, where the plugin lives."""

    class Page(http.server.BaseHTTPRequestHandler):
        def _caller(self) -> meridian.Caller | None:
            presented = self.headers.get_all("Meridian-Caller") or []
            if len(presented) != 1:
                self._send(401, "Open this page from the dashboard.", "text/plain")
                return None
            return meridian.Caller.from_header(presented[0])

        def _send(self, status: int, body: str, kind: str = "text/html") -> None:
            data = body.encode()
            self.send_response(status)
            self.send_header("Content-Type", f"{kind}; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self) -> None:  # noqa: N802 - the server's name
            caller = self._caller()
            if caller is not None:
                self._send(200, render(caller))

        def do_POST(self) -> None:  # noqa: N802
            caller = self._caller()
            if caller is None:
                return
            self.rfile.read(int(self.headers.get("Content-Length") or 0))
            if self.path != "/statement":
                self._send(404, "No such action.", "text/plain")
                return
            # Sent for the person: the sidecar admits it only if they may
            # write through this plugin, and the store records it as theirs.
            opening = plugin.record_holdings_statement(
                source="reference",
                external_statement_id=f"reference-{uuid.uuid4()}",
                as_of_date=time.strftime("%Y-%m-%d", time.gmtime()),
                read_at_ns=time.time_ns(),
                expected_rows=0,
                acting_for=caller.header,
            )
            try:
                opened = asyncio.run_coroutine_threadsafe(opening, loop).result(timeout=15)
                notice = f"Opened statement {opened.statement_id} for you."
            except meridian.MeridianError as refused:
                notice = f"Refused: {refused}"
            self._send(200, render(caller, notice))

        def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server
