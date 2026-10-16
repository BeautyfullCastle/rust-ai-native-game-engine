"use strict";
// One browser client of the end-to-end proof (crates/orr_server/tests/browser_e2e.rs):
// opens crates/orr_web/web/index.html in headless Chromium, joins the room the test
// started, plays the scripted bot until TICKS ticks are verified, prints
//   BROWSER key=value ...
//   CHECKSUMS tick:hex,tick:hex,...
// and exits 0. Exit code 2 = skipped (no Playwright / no wasm package) unless ORR_REQUIRE_BROWSER is set.
//
// The page runs the client in a Web Worker; window.orr is the page-side view of it (cached reports).
// Env: UDP (WebTransport port), WS (WebSocket port), HASH (cert sha-256 hex), MODE (auto|webtransport|websocket),
//      (WS may also be a full ws:// or wss:// URL), ROOM, BUILD (default: the game's), GAME (arena|phys, default arena),
//      TICKS (default 600), SLOT (optional), TIMEOUT_MS (default 60000), CHROMIUM, CHROMIUM_FLAGS.
//      RENDER (auto|webgpu|webgl|2d: the page's view, default auto = WebGPU, WebGL2, then a 2D canvas), SCREENSHOT (png path).
// Also prints RENDERER (the view the page ended up with), FRAMES (page frame and draw times) and SIM (worker time per tick call).

const fs = require("fs");
const path = require("path");
const lib = require("./lib.cjs");
const { checkBuildIdBoundary } = require("./build_id_boundary.cjs");
const { browserOptionsFromEnvironment } = require("./deployment_manifest.cjs");

const skip = (why) => { console.log((lib.requiresBrowser() ? "ERROR: " : "SKIP: ") + why); process.exit(lib.unavailableExitCode()); };

(async () => {
  const pw = lib.loadPlaywright();
  if (!pw) skip("playwright not found (set NODE_PATH or npm i -g playwright)");
  const web = path.join(lib.repoRoot(), "crates", "orr_web", "web");
  if (!fs.existsSync(path.join(web, "pkg", "orr_web.js"))) skip("crates/orr_web/web/pkg is missing (run tools/build_web.sh)");
  const env = process.env;
  const deployment = browserOptionsFromEnvironment(env);
  const ticks = +(env.TICKS || 600);
  const { server, port: httpPort } = await lib.serveStatic(web);
  let browser;
  try {
    browser = await pw.chromium.launch(lib.chromiumOptions());
  } catch (e) {
    server.close();
    if (lib.requiresBrowser()) console.error(e);
    skip("cannot launch Chromium: " + String(e).split("\n")[0]);
  }
  let code = 1;
  try {
    const page = await browser.newPage();
    page.on("console", (m) => console.log("[page]", m.text()));
    page.on("pageerror", (e) => console.log("[pageerror]", String(e)));
    const q = new URLSearchParams({ auto: "1", bot: "1", mode: env.MODE || "auto", room: env.ROOM || "1", game: deployment ? deployment.game : (env.GAME || "arena"), render: env.RENDER || "auto" });
    if (deployment) {
      q.set("build", deployment.buildId);
      console.log(`DEPLOYMENT game=${deployment.game} build_id=${deployment.buildId}`);
    } else if (env.BUILD) q.set("build", env.BUILD);
    if (env.UDP) q.set("wt", `127.0.0.1:${env.UDP}`);
    if (env.HASH) q.set("hash", env.HASH);
    if (env.WS) q.set("ws", /^wss?:/.test(env.WS) ? env.WS : `127.0.0.1:${env.WS}`);
    if (env.SLOT) q.set("slot", env.SLOT);
    await page.goto(`http://localhost:${httpPort}/index.html?${q}`);
    const boundary = await page.evaluate(checkBuildIdBoundary);
    console.log("BUILD_ID_BOUNDARY " + JSON.stringify(boundary));
    const deadline = Date.now() + +(env.TIMEOUT_MS || 60000);
    let r = null;
    while (Date.now() < deadline) {
      r = await page.evaluate(() => (window.orr ? window.orr.report() : null)).catch(() => null);
      if (r && (r.verified_tick >= ticks || /^(rejected|failed|Disconnected)/.test(r.state))) break;
      await new Promise((res) => setTimeout(res, 250));
    }
    if (!r) throw new Error("the page never started a client");
    const err = await page.evaluate(() => window.orr.transportError()).catch(() => "");
    const over = await page.evaluate(() => window.orr.oversize()).catch(() => 0);
    const frames = await page.evaluate(() => window.orr.frameStats()).catch(() => null);
    const sim = await page.evaluate(() => window.orr.simStats()).catch(() => null);
    const rend = await page.evaluate(async () => { await window.orrGpu.ready; return window.orrGpu.renderer(); }).catch(() => null);
    const werr = await page.evaluate(() => window.orr.error()).catch(() => "");
    if (werr) console.log("WORKER_ERROR " + werr);
    console.log(`BROWSER state=${JSON.stringify(r.state)} transport=${r.transport} slot=${r.slot} rollbacks=${r.rollbacks} resim_ticks=${r.resim_ticks} ` +
      `max_prediction_depth=${r.max_prediction_depth} desyncs=${r.desyncs} decode_errors=${r.decode_errors} head=${r.head_tick} verified=${r.verified_tick} ` +
      `rtt_us=${r.rtt_us} delay=${r.delay} oversize_to_reliable=${over} transport_error=${JSON.stringify(err)}`);
    const fmt = (o) => Object.entries(o).map(([k, v]) => `${k}=${typeof v === "number" ? +v.toFixed(3) : v}`).join(" ");
    if (rend) console.log("RENDERER " + Object.entries(rend).map(([k, v]) => `${k}=${JSON.stringify(v)}`).join(" "));
    if (frames) console.log("FRAMES " + fmt(frames));
    if (sim) console.log("SIM " + fmt(sim));
    console.log("CHECKSUMS "+ r.checksums.map(([t, c]) => `${t}:${c}`).join(","));
    const expectedReject = env.EXPECT_REJECT || "";
    if (expectedReject) {
      const mismatch = /^rejected: BuildHashMismatch \{ server: ([0-9]+), client: ([0-9]+) \}$/.exec(r.state);
      const isExpectedBuildMismatch = expectedReject === "build_hash_mismatch" && mismatch && r.verified_tick === 0 &&
        env.EXPECTED_SERVER_HASH === mismatch[1] && env.EXPECTED_CLIENT_HASH === mismatch[2];
      if (isExpectedBuildMismatch) {
        console.log(`DEPLOYMENT_REJECT kind=build_hash_mismatch server=${mismatch[1]} client=${mismatch[2]} verified=${r.verified_tick}`);
        code = 0;
      } else {
        console.error(`expected ${expectedReject} rejection, got state=${JSON.stringify(r.state)} verified=${r.verified_tick}`);
        code = 1;
      }
    } else {
      code = r.verified_tick >= ticks ? 0 : 1;
    }
    // SCREENSHOT=/path.png: the page's canvas as drawn at the end of the run (the test counts its colored pixels).
    if (env.SCREENSHOT) await page.locator(rend && rend.kind !== "2d" ? "#g" : "#c").screenshot({ path: env.SCREENSHOT }).catch((e) => console.log("screenshot failed", String(e)));
    await page.evaluate(() => { window.orr.leave(); }).catch(() => {});
    await new Promise((res) => setTimeout(res, 300));
  } catch (e) {
    console.error(e);
  } finally {
    await browser.close();
    server.close();
  }
  process.exit(code);
})().catch((e) => { console.error(e); process.exit(1); });
