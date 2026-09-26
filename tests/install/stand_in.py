"""A stand-in for GitHub's releases, for tests/install/run.sh.

Serves what the install script and `meridian upgrade` ask GitHub for, and no
more: /releases/latest redirecting to a tag, and each binary and its
.sha256 under /releases/download/<tag>/. v9.9.9 is the latest and whole;
v6.6.6's checksums are wrong. Each binary is a two-line script saying which
release and target it is.
"""

import hashlib
import http.server
import sys

PORT = int(sys.argv[1])
TARGETS = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
]
FILES = {}
for tag in ("v9.9.9", "v6.6.6"):
    for target in TARGETS:
        body = f"#!/bin/sh\necho 'meridian {tag[1:]} (stand-in {target})'\n".encode()
        digest = hashlib.sha256(body).hexdigest()
        if tag == "v6.6.6":
            digest = hashlib.sha256(b"something else").hexdigest()
        name = f"meridian-{target}"
        FILES[f"/releases/download/{tag}/{name}"] = body
        # As the release workflow writes it: the file name after the hash.
        FILES[f"/releases/download/{tag}/{name}.sha256"] = f"{digest}  {name}\n".encode()


class Releases(http.server.BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - the server's name
        if self.path == "/releases/latest":
            self.send_response(302)
            self.send_header("Location", f"http://127.0.0.1:{PORT}/releases/tag/v9.9.9")
            self.end_headers()
            return
        if self.path == "/releases/tag/v9.9.9":
            body = b"the release page"
        elif self.path in FILES:
            body = FILES[self.path]
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Releases).serve_forever()
