"use strict";
// Headless Chromium trial of WebTransport (streams + datagrams) and WebSocket
// against `orr_net`'s wt_echo example. Prints the page's JSON report.
//
//   cargo build -p orr_net --release --example wt_echo
//   node tools/webtransport/trial.cjs
//
// Env: PLAYWRIGHT via NODE_PATH or the global npm root; CHROMIUM=/path/to/chrome;
// CHROMIUM_FLAGS="--flag ..."; N=200 pings; WT_ECHO=path to the example binary; WT_ECHO_ARGS=extra wt_echo flags;
// HASH=override of the certificate hash (empty = no serverCertificateHashes, for CA certificates).
// Exit code 0 when WebTransport (stream + datagrams) and WebSocket all worked.

const path = require("path");
const lib = require("./lib.cjs");

(async () => {
  const pw = lib.loadPlaywright();
  if (!pw) { console.log("SKIP: playwright not found (set NODE_PATH or npm i -g playwright)"); process.exit(process.env.ORR_REQUIRE_BROWSER ? 1 : 0); }
  const echo = process.env.WT_ECHO || path.join(lib.repoRoot(), "target", "release", "examples", "wt_echo");
  const { proc, ready } = await lib.spawnReady(echo, (process.env.WT_ECHO_ARGS || "").split(/\s+/).filter(Boolean));
  const { server, port: httpPort } = await lib.serveStatic(__dirname);
  const browser = await pw.chromium.launch(lib.chromiumOptions());
  let code = 1;
  try {
    const page = await browser.newPage();
    page.on("console", (m) => console.log("[page]", m.text()));
    const n = process.env.N || "200";
    await page.goto(`http://localhost:${httpPort}/trial.html?host=127.0.0.1&port=${ready.udp_port}&wsport=${ready.ws_port}&hash=${process.env.HASH !== undefined ? process.env.HASH : ready.hash || ""}&n=${n}&auto=1`);
    await page.waitForFunction(() => window.trialResult, null, { timeout: 120000 });
    const r = await page.evaluate(() => window.trialResult);
    console.log("RESULT " + JSON.stringify(r, null, 2));
    const wt = r.webtransport || {};
    code = wt.connected && wt.stream && wt.stream.received === +n && wt.datagram && wt.datagram.received > 0 && r.websocket && r.websocket.stream && r.websocket.stream.received === +n ? 0 : 1;
  } finally {
    await browser.close();
    server.close();
    proc.stdin.end();
    setTimeout(() => proc.kill(), 500);
  }
  console.log(code === 0 ? "TRIAL OK" : "TRIAL FAILED");
  process.exit(code);
})().catch((e) => { console.error(e); process.exit(1); });
