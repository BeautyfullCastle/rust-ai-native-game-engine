"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");
const { browserOptionsFromEnvironment, loadDeploymentManifest } = require("./deployment_manifest.cjs");
const { repoRoot } = require("./lib.cjs");

const python = process.env.ORR_PYTHON || (process.platform === "win32" ? "python" : "python3");
const tool = path.join(repoRoot(), "tools", "deployment_manifest.py");

function createManifest(dir, name, rawId) {
  const output = path.join(dir, `${name}.json`);
  const result = spawnSync(python, [tool, "create", "--game", "arena", "--game-code-identity", name,
    "--game-code-id", rawId, "--output", output], {
    encoding: "utf8", shell: false, timeout: 5000, maxBuffer: 1024 * 1024,
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, result.stderr || result.stdout);
  return output;
}

test("browser manifest passes the exact build id string from the Python launch plan", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "orr-deployment-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const manifestPath = createManifest(dir, "large-id", "9007199254740993");
  const plan = loadDeploymentManifest(manifestPath);
  assert.equal(typeof plan.browser.opts.build_id, "string");
  assert.match(plan.browser.opts.build_id, /^[1-9][0-9]*$/);
  assert.ok(BigInt(plan.browser.opts.build_id) > BigInt(Number.MAX_SAFE_INTEGER));
  assert.deepEqual(browserOptionsFromEnvironment({ DEPLOYMENT_MANIFEST: manifestPath }), {
    game: "arena", buildId: plan.browser.query.build,
  });
  assert.deepEqual(browserOptionsFromEnvironment({ DEPLOYMENT_MANIFEST: manifestPath, GAME: "arena", BUILD: plan.browser.query.build }), {
    game: "arena", buildId: plan.browser.query.build,
  });
  assert.throws(() => browserOptionsFromEnvironment({ DEPLOYMENT_MANIFEST: manifestPath, GAME: "phys" }), /GAME conflicts/);
  assert.throws(() => browserOptionsFromEnvironment({ DEPLOYMENT_MANIFEST: manifestPath, BUILD: "9007199254740993" }), /BUILD conflicts/);
});

test("invalid, zero-id, and wrong-frame-version manifests fail through Python validation", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "orr-deployment-invalid-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const valid = createManifest(dir, "valid", "9007199254740993");
  const original = JSON.parse(fs.readFileSync(valid, "utf8"));

  const zero = path.join(dir, "zero.json");
  fs.writeFileSync(zero, JSON.stringify({ ...original, game_code_id: "0", build_id: "0" }));
  assert.throws(() => loadDeploymentManifest(zero), /deployment manifest export failed/);

  const wrongVersion = path.join(dir, "wrong-version.json");
  fs.writeFileSync(wrongVersion, JSON.stringify({ ...original, frame_format_version: original.frame_format_version + 1 }));
  assert.throws(() => loadDeploymentManifest(wrongVersion), /deployment manifest export failed/);

  const malformed = path.join(dir, "malformed.json");
  fs.writeFileSync(malformed, "{}");
  assert.throws(() => loadDeploymentManifest(malformed), /deployment manifest export failed/);
});
