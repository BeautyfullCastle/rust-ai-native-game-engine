"use strict";
// Runs the orr_wasm_bench page (crates/orr_wasm_bench/web) in headless Chromium and prints
//   RESULT <name> <ns_per_iter> <ns_per_unit> <iters> <checksum>
// like `bench_runner`. Needs the wasm package: tools/wasm_bench.sh builds it into web/pkg.
// Env: MS (target ms per case, default 300), ONLY (comma separated substrings), CHROMIUM, CHROMIUM_FLAGS, NODE_PATH.

const fs = require("fs");
const path = require("path");
const lib = require("./webtransport/lib.cjs");

(async () => {
  const pw = lib.loadPlaywright();
  if (!pw) { console.log("SKIP: playwright not found"); process.exit(2); }
  const web = path.join(lib.repoRoot(), "crates", "orr_wasm_bench", "web");
  if (!fs.existsSync(path.join(web, "pkg", "orr_wasm_bench.js"))) { console.log("SKIP: build the package first (tools/wasm_bench.sh)"); process.exit(2); }
  const { server, port } = await lib.serveStatic(web);
  const browser = await pw.chromium.launch(lib.chromiumOptions());
  let code = 1;
  try {
    const page = await browser.newPage();
    page.on("pageerror", (e) => { console.log("[pageerror]", String(e)); process.exit(1); });
    const q = new URLSearchParams({ ms: process.env.MS || "300" });
    if (process.env.ONLY) q.set("only", process.env.ONLY);
    await page.goto(`http://127.0.0.1:${port}/index.html?${q}`);
    await page.waitForFunction(() => window.benchResults, null, { timeout: 15 * 60 * 1000, polling: 500 });
    const res = await page.evaluate(() => window.benchResults);
    console.log("TARGET chromium " + browser.version());
    for (const r of res) console.log(`RESULT ${r.name} ${r.ns_per_iter.toFixed(3)} ${r.ns_per_unit.toFixed(3)} ${r.iters} ${r.checksum}`);
    code = 0;
  } catch (e) {
    console.error(e);
  } finally {
    await browser.close();
    server.close();
  }
  process.exit(code);
})();
