// An HTTPS server and a client for it, on Node, for run.sh.
//
//   node https.js serve <cert> <key> <portfile>   on 127.0.0.1 alone
//   node https.js get <url>                       exits 0 on a 200
//
// The client is what an app on Node, the Claude app among them, does: Node's
// own roots plus whatever NODE_EXTRA_CA_CERTS names when it starts, and
// nothing from the keychain. The name, under .localhost, is resolved to
// 127.0.0.1 here, since macOS's resolver does not answer for names under
// .localhost the way a browser does; the certificate is still checked
// against the name in the URL.

const fs = require("fs");
const https = require("https");

const [mode, ...rest] = process.argv.slice(2);

if (mode === "serve") {
  const [cert, key, portfile] = rest;
  const server = https.createServer(
    { cert: fs.readFileSync(cert), key: fs.readFileSync(key) },
    (_request, response) => response.end("authority-e2e ok\n"),
  );
  server.listen(0, "127.0.0.1", () => {
    fs.writeFileSync(portfile, String(server.address().port));
  });
} else if (mode === "get") {
  const loopback = (_host, options, callback) => {
    if (typeof options === "function") {
      callback = options;
      options = {};
    }
    if (options && options.all) {
      callback(null, [{ address: "127.0.0.1", family: 4 }]);
    } else {
      callback(null, "127.0.0.1", 4);
    }
  };
  https
    .get(rest[0], { lookup: loopback, timeout: 10000 }, (response) => {
      let body = "";
      response.on("data", (chunk) => (body += chunk));
      response.on("end", () => {
        console.log(`${response.statusCode} ${body.trim()}`);
        process.exit(response.statusCode === 200 ? 0 : 1);
      });
    })
    .on("error", (failed) => {
      console.log(`refused: ${failed.code || failed.message}`);
      process.exit(1);
    });
} else {
  console.error("usage: node https.js serve <cert> <key> <portfile> | get <url>");
  process.exit(2);
}
