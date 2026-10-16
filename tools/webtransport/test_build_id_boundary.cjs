"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { checkBuildIdBoundary } = require("./build_id_boundary.cjs");
const { repoRoot } = require("./lib.cjs");

test("real generated JS/WASM rejects rounded Numbers and preserves all u64 string bits", async () => {
  // Deliberately never skip: npm test is run only after package generation in CI.
  const pkg = path.join(repoRoot(), "crates/orr_web/web/pkg");
  const result = await checkBuildIdBoundary({
    moduleUrl: pathToFileURL(path.join(pkg, "orr_web.js")).href,
    moduleBytes: fs.readFileSync(path.join(pkg, "orr_web_bg.wasm")),
  });
  assert.deepEqual(result, { constructors: 2, accepted: 30, rejected: 54 });
});
