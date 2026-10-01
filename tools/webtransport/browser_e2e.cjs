"use strict";
// One browser client of the end-to-end proof (crates/orr_server/tests/browser_e2e.rs):
// opens crates/orr_web/web/index.html in headless Chromium, joins the room the test
// started, plays the scripted bot until TICKS ticks are verified, prints
//   BROWSER key=value ...
//   CHECKSUMS tick:hex,tick:hex,...
// and exits 0. Exit code 2 = skipped (no Playwright / no wasm package) unless ORR_REQUIRE_BROWSER is set.
//
// Env: UDP (WebTransport port), WS (WebSocket port), HASH (cert sha-256 hex), MODE (auto|webtransport|websocket),
//      (WS may also be a full ws:// or wss:// URL), ROOM, BUILD, TICKS (default 600), SLOT (optional), TIMEOUT_MS (default 60000), CHROMIUM, CHROMIUM_FLAGS.

const fs = require("fs");
const path = require("path");
const lib = require("./lib.cjs");

const skip = (why) => { console.log("SKIP: " + why); process.exit(process.env.ORR_REQUIRE_BROWSER ? 1 : 2); };

(async () => {
  const pw = lib.loadPlaywright();
  if (!pw) skip("playwright not found (set NODE_PATH or npm i -g playwright)");
  const web = path.join(lib.repoRoot(), "crates", "orr_web", "web");
  if (!fs.existsSync(path.join(web, "pkg", "orr_web.js"))) skip("crates/orr_web/web/pkg is missing (run tools/build_web.sh)");
  const env = process.env;
  const ticks = +(env.TICKS || 600);
  const { server, port: httpPort } = await lib.serveStatic(web);
  let browser;
  try {
    browser = await pw.chromium.launch(lib.chromiumOptions());
  } catch (e) {
    server.close();
    skip("cannot launch Chromium: " + String(e).split("\n")[0]);
  }
  let code = 1;
  try {
    const page = await browser.newPage();
    page.on("console", (m) => console.log("[page]", m.text()));
    page.on("pageerror", (e) => console.log("[pageerror]", String(e)));
    const q = new URLSearchParams({ auto: "1", bot: "1", mode: env.MODE || "auto", room: env.ROOM || "1", build: env.BUILD || "1" });
    if (env.UDP) q.set("wt", `127.0.0.1:${env.UDP}`);
    if (env.HASH) q.set("hash", env.HASH);
    if (env.WS) q.set("ws", /^wss?:/.test(env.WS) ? env.WS : `127.0.0.1:${env.WS}`);
    if (env.SLOT) q.set("slot", env.SLOT);
    await page.goto(`http://localhost:${httpPort}/index.html?${q}`);
    const deadline = Date.now() + +(env.TIMEOUT_MS || 60000);
    let r = null;
    while (Date.now() < deadline) {
      r = await page.evaluate(() => (window.orr ? window.orr.report() : null)).catch(() => null);
      if (r && (r.verified_tick >= ticks || /^(rejected|failed|Disconnected)/.test(r.state))) break;
      await new Promise((res) => setTimeout(res, 250));
    }
    if (!r) throw new Error("the page never started a client");
    const err = await page.evaluate(() => window.orr.client.transport_error()).catch(() => "");
    const over = await page.evaluate(() => window.orr.client.oversize_to_reliable()).catch(() => 0);
    console.log(`BROWSER state=${JSON.stringify(r.state)} transport=${r.transport} slot=${r.slot} rollbacks=${r.rollbacks} resim_ticks=${r.resim_ticks} ` +
      `max_prediction_depth=${r.max_prediction_depth} desyncs=${r.desyncs} decode_errors=${r.decode_errors} head=${r.head_tick} verified=${r.verified_tick} ` +
      `rtt_us=${r.rtt_us} delay=${r.delay} oversize_to_reliable=${over} transport_error=${JSON.stringify(err)}`);
    console.log("CHECKSUMS " + r.checksums.map(([t, c]) => `${t}:${c}`).join(","));
    code = r.verified_tick >= ticks ? 0 : 1;
    await page.evaluate(() => { window.orr.client.leave(); }).catch(() => {});
    await new Promise((res) => setTimeout(res, 300));
  } catch (e) {
    console.error(e);
  } finally {
    await browser.close();
    server.close();
  }
  process.exit(code);
})().catch((e) => { console.error(e); process.exit(1); });
