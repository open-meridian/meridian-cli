"""The plugin's page, as people see it through the deployment's dashboard.

Served on loopback, where only this plugin's sidecar reaches it: the sidecar
verified who is asking before forwarding the request, and says so in one
`Meridian-Caller` header, which `meridian.Caller` reads. Nothing here checks a
signature, holds a key or keeps a session -- that is the point.

It shows who is asking and what they may see through this plugin, tag by tag,
and offers one action that writes for them: opening an empty holdings
statement, sent with `acting_for` so the sidecar decides whether they may.
Replace it with your plugin's own pages; keep reading the caller the same way.

Built on the kit (AGENTS.md says how): the dashboard serves Open Meridian's UI
kit at /.meridian/ui/<version>/ on this plugin's own host, and the page links
its stylesheet and script, uses its classes and its components, and writes no
colour and no theme code of its own. The dashboard's frame draws the plugin's
name and the person; the page draws only its content.

Where the kit is not served (a dashboard that does not serve it yet, or the
page opened some other way) the page still works, unstyled: the table is in
the HTML inside <om-grid>, which a browser shows as it is until the kit's grid
replaces it; the grid's data sits beside it as JSON, read only once the grid
is defined; and the action is a plain form. Nothing waits on the kit.

The standard library's server, so the plugin needs nothing its base image
does not already have. A framework on ASGI can use `meridian.CallerMiddleware`
instead.
"""

from __future__ import annotations

import asyncio
import html
import http.server
import json
import threading
import time
import uuid
from typing import Any

import meridian

TITLE = "Reference plugin"

# The kit's version this page was built against. The dashboard serves the
# deployment's; pinning one keeps the page as it was built.
KIT = "/.meridian/ui/0.1.0/"

# The grid's columns and rows, set by the page's script once the kit has
# defined the grid. Without the kit it never is, and the table stays.
_GRID = """
customElements.whenDefined("om-grid").then(() => {
  const grid = document.getElementById("access");
  grid.columns = [
    { key: "tag", label: "Tag", type: "code" },
    { key: "read", label: "Read" },
    { key: "write", label: "Write" },
  ];
  grid.setRows(JSON.parse(document.getElementById("access-rows").textContent));
});
"""


def _script_json(value: object) -> str:
    """JSON safe inside a <script> element: nothing in it can close the element."""
    text = json.dumps(value)
    return text.replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")


def render(caller: meridian.Caller, notice: str = "", refused: bool = False) -> str:
    rows = [
        {
            "tag": held.tag,
            "read": ", ".join(sorted(held.read)) or "none",
            "write": ", ".join(sorted(held.write)) or "none",
        }
        for held in caller.access
    ]
    cells = [[html.escape(row[k]) for k in ("tag", "read", "write")] for row in rows]
    fallback = "".join(
        f"<tr><td><code>{tag}</code></td><td>{read}</td><td>{write}</td></tr>"
        for tag, read, write in cells
    )
    if not fallback:
        fallback = '<tr><td colspan="3">Nothing is granted to you here.</td></tr>'
    said = (
        f'<div class="notice {"bad" if refused else "good"}" role="status">'
        f"{html.escape(notice)}</div>"
        if notice
        else ""
    )
    return (
        '<!doctype html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>{TITLE}</title>"
        f'<link rel="stylesheet" href="{KIT}meridian.css">'
        f'<script src="{KIT}meridian.js"></script>'
        "</head><body>"
        '<main class="page">'
        '<header class="page-head"><div>'
        f"<h1>{TITLE}</h1>"
        f"<p>Signed in as <strong>{html.escape(caller.display_name)}</strong>.</p>"
        "</div>"
        '<div class="actions"><form class="inline" method="post" action="/statement">'
        '<button class="primary">Open an empty statement for me</button></form></div>'
        "</header>"
        f"{said}"
        '<section class="panel">'
        '<div class="panel-body"><h2>What you may see here</h2>'
        '<p class="muted">Accounts, tag by tag, that you may read or write here.</p>'
        "</div>"
        '<om-grid id="access" row-key="tag" caption="What you may see here"'
        ' empty="Nothing is granted to you here.">'
        "<table><thead><tr><th>Tag</th><th>Read</th><th>Write</th></tr></thead>"
        f"<tbody>{fallback}</tbody></table>"
        "</om-grid>"
        f'<script type="application/json" id="access-rows">{_script_json(rows)}</script>'
        "</section>"
        "</main>"
        f'<script type="module">{_GRID}</script>'
        "</body></html>"
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
                said = f"Opened statement {opened.statement_id} for you."
                self._send(200, render(caller, said))
            except meridian.MeridianError as refused:
                self._send(200, render(caller, f"Refused: {refused}", refused=True))

        def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server
