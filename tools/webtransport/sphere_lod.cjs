"use strict";
// Runs isolated Renderer3D sphere-LOD cases on one explicitly selected browser backend.

const fs = require("fs");
const path = require("path");
const lib = require("./lib.cjs");

const backend = process.env.SPHERE_LOD_BACKEND || "";
const required = process.env.ORR_REQUIRE_BROWSER === "1" || process.env.ORR_REQUIRE_GPU === "1" ||
  (backend === "webgpu" && process.env.ORR_REQUIRE_WEBGPU === "1");
class SkipError extends Error {}
const skip = (why) => { throw new SkipError(why); };

const flags = {
  webgl: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
  // Windows WebGPU needs ANGLE's D3D11 adapter even when Dawn uses SwiftShader.
  // Chromium's webgpu-swiftshader test preset keeps ANGLE initialized for it.
  webgpu: process.platform === "win32"
    ? ["--enable-unsafe-webgpu", "--use-webgpu-adapter=swiftshader", "--use-gpu-in-tests"]
    : ["--enable-unsafe-webgpu", "--enable-unsafe-swiftshader", "--use-webgpu-adapter=swiftshader", "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--use-angle=swiftshader"],
};

(async () => {
  if (!(backend in flags)) throw new Error("SPHERE_LOD_BACKEND must be webgpu or webgl");
  const pw = lib.loadPlaywright();
  if (!pw) skip("Playwright not found (set NODE_PATH to its runtime installation)");
  const root = lib.repoRoot();
  const web = path.join(root, "crates", "orr_web", "web");
  if (!fs.existsSync(path.join(web, "pkg_gpu", "orr_web_gpu.js"))) skip("GPU wasm package is missing (run tools/build_web.sh)");
  const output = process.env.SPHERE_LOD_SCREENSHOTS;
  if (!output) throw new Error("SPHERE_LOD_SCREENSHOTS must name an output directory");
  fs.mkdirSync(output, { recursive: true });

  const { server, port } = await lib.serveStatic(web);
  let browser;
  try {
    browser = await pw.chromium.launch(lib.chromiumOptions(flags[backend]));
  } catch (error) {
    server.close();
    skip(`cannot launch Chromium: ${String(error).split("\n")[0]}`);
  }

  try {
    const probe = await browser.newPage({ viewport: { width: 256, height: 256 }, deviceScaleFactor: 1 });
    await probe.goto(`http://127.0.0.1:${port}/sphere_lod.html?probe=1`);
    const available = await probe.evaluate(async (which) => {
      if (which === "webgpu") {
        if (!navigator.gpu) return false;
        return !!(await navigator.gpu.requestAdapter());
      }
      return !!document.createElement("canvas").getContext("webgl2");
    }, backend);
    await probe.close();
    if (!available) skip(`Chromium has no ${backend} adapter/context`);

    const expected = backend === "webgpu" ? "BrowserWebGpu" : "Gl";
    for (const preset of ["default", "low"]) {
      for (const scene of ["near", "far", "mixed"]) {
        for (const lod of [false, true]) {
          const page = await browser.newPage({ viewport: { width: 256, height: 256 }, deviceScaleFactor: 1 });
          const params = new URLSearchParams({ backend, preset, scene, lod: lod ? "1" : "0" });
          await page.goto(`http://127.0.0.1:${port}/sphere_lod.html?${params}`);
          await page.waitForFunction(() => window.sphereLodResult || window.sphereLodError, undefined, { timeout: 30000 });
          const result = await page.evaluate(() => ({
            result: window.sphereLodResult || null,
            error: window.sphereLodError || "",
          }));
          if (result.error) throw new Error(`${preset}/${scene}/lod=${lod}: ${result.error}`);
          if (!result.result) throw new Error(`${preset}/${scene}/lod=${lod}: missing fixture result`);
          if (result.result.actual_backend !== expected) {
            throw new Error(`requested ${expected}, got ${result.result.actual_backend}`);
          }
          if (!result.result.drawn) throw new Error(`${preset}/${scene}/lod=${lod}: draw was skipped`);
          const key = `${preset}_${scene}_${lod ? "lod" : "fixed"}`;
          const screenshot = path.join(output, `${key}.png`);
          await page.locator("#sphere").screenshot({ path: screenshot });
          console.log("RESULT " + JSON.stringify({ key, screenshot, ...result.result }));
          await page.close();
        }
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
})().catch((error) => {
  if (error instanceof SkipError) {
    console.log((required ? "ERROR: " : "SKIP: ") + error.message);
    process.exitCode = required ? 1 : 2;
  } else {
    console.error("ERROR: " + (error && error.stack || error));
    process.exitCode = 1;
  }
});
