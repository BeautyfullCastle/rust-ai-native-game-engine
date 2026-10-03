"use strict";
const assert = require("node:assert/strict");
const test = require("node:test");
const { requiresBrowser, unavailableExitCode, chromiumOptions } = require("./lib.cjs");

test("missing browser prerequisites fail closed whenever browser or WebGPU is required", () => {
  for (const browser of [undefined, "", "0", "1", "true"]) {
    for (const gpu of [undefined, "", "0", "1", "true"]) {
      const env = { ORR_REQUIRE_BROWSER: browser, ORR_REQUIRE_WEBGPU: gpu };
      const required = browser === "1" || gpu === "1";
      assert.equal(requiresBrowser(env), required);
      assert.equal(unavailableExitCode(env), required ? 1 : 2);
    }
  }
});

test("Chromium options preserve caller flags and optional executable", () => {
  const previous = { flags: process.env.CHROMIUM_FLAGS, executable: process.env.CHROMIUM };
  try {
    process.env.CHROMIUM_FLAGS = " --flag-one  --flag-two ";
    process.env.CHROMIUM = "/example/chromium";
    assert.deepEqual(chromiumOptions(["--view-flag"]), {
      headless: true,
      args: ["--view-flag", "--flag-one", "--flag-two"],
      executablePath: "/example/chromium",
    });
  } finally {
    if (previous.flags === undefined) delete process.env.CHROMIUM_FLAGS;
    else process.env.CHROMIUM_FLAGS = previous.flags;
    if (previous.executable === undefined) delete process.env.CHROMIUM;
    else process.env.CHROMIUM = previous.executable;
  }
});

test("the executable runner cannot silently skip missing Playwright, WASM, or Chromium in required mode", () => {
  const fs = require("node:fs");
  const os = require("node:os");
  const path = require("node:path");
  const { spawnSync } = require("node:child_process");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "orr-browser-prerequisites-"));
  try {
    for (const file of ["browser_e2e.cjs", "build_id_boundary.cjs", "deployment_manifest.cjs"]) {
      fs.copyFileSync(path.join(__dirname, file), path.join(dir, file));
    }
    const realLib = JSON.stringify(path.join(__dirname, "lib.cjs"));
    for (const missing of ["playwright", "wasm", "chromium"]) {
      const root = path.join(dir, missing);
      const pkg = path.join(root, "crates/orr_web/web/pkg");
      fs.mkdirSync(pkg, { recursive: true });
      if (missing !== "wasm") fs.writeFileSync(path.join(pkg, "orr_web.js"), "");
      fs.writeFileSync(path.join(dir, "lib.cjs"), `
        module.exports = {
          ...require(${realLib}),
          repoRoot: () => ${JSON.stringify(root)},
          loadPlaywright: () => ${missing === "playwright" ? "null" : '{ chromium: { launch: async () => { throw new Error("fixture Chromium unavailable"); } } }'},
          serveStatic: async () => ({ server: { close() {} }, port: 0 }),
        };
      `);
      for (const requirements of [
        {}, { ORR_REQUIRE_BROWSER: "0", ORR_REQUIRE_WEBGPU: "0" },
        { ORR_REQUIRE_BROWSER: "1" }, { ORR_REQUIRE_WEBGPU: "1" },
        { ORR_REQUIRE_BROWSER: "1", ORR_REQUIRE_WEBGPU: "1" },
      ]) {
        const env = { ...process.env };
        delete env.ORR_REQUIRE_BROWSER;
        delete env.ORR_REQUIRE_WEBGPU;
        delete env.DEPLOYMENT_MANIFEST;
        Object.assign(env, requirements);
        const child = spawnSync(process.execPath, [path.join(dir, "browser_e2e.cjs")], { env, encoding: "utf8", timeout: 5000 });
        assert.equal(child.error, undefined);
        assert.equal(child.status, unavailableExitCode(env), `${missing}: ${child.stdout}${child.stderr}`);
        assert.match(child.stdout, requiresBrowser(env) ? /^ERROR: / : /^SKIP: /);
      }
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
