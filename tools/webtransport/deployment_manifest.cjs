"use strict";

// The Python implementation owns the pinned deployment-manifest format. Keep
// the browser runner as a thin consumer so it cannot silently drift from it.
const path = require("node:path");
const { spawnSync } = require("node:child_process");

function pythonCommand() {
  return process.env.ORR_PYTHON || (process.platform === "win32" ? "python" : "python3");
}

function loadDeploymentManifest(manifestPath) {
  const script = path.resolve(__dirname, "..", "deployment_manifest.py");
  const result = spawnSync(pythonCommand(), [script, "export", path.resolve(manifestPath)], {
    encoding: "utf8",
    shell: false,
    timeout: 5000,
    maxBuffer: 1024 * 1024,
  });
  if (result.error) throw new Error(`cannot export deployment manifest: ${result.error.message}`);
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || `exit ${result.status}`).trim();
    throw new Error(`deployment manifest export failed: ${detail}`);
  }
  let plan;
  try {
    plan = JSON.parse(result.stdout);
  } catch (e) {
    throw new Error(`deployment manifest export returned invalid JSON: ${e.message}`);
  }
  const buildId = plan && plan.browser && plan.browser.opts && plan.browser.opts.build_id;
  const query = plan && plan.browser && plan.browser.query;
  if (typeof buildId !== "string" || !/^[1-9][0-9]*$/.test(buildId)) {
    throw new Error("deployment manifest export has no decimal string browser build_id");
  }
  if (!query || typeof query.game !== "string" || typeof query.build !== "string") {
    throw new Error("deployment manifest export has no browser query game/build strings");
  }
  if (query.build !== buildId || !["arena", "phys"].includes(query.game)) {
    throw new Error("deployment manifest browser query conflicts with its browser options");
  }
  return plan;
}

function browserOptionsFromEnvironment(env = process.env) {
  if (!env.DEPLOYMENT_MANIFEST) return null;
  const plan = loadDeploymentManifest(env.DEPLOYMENT_MANIFEST);
  const { game, build: buildId } = plan.browser.query;
  if (env.GAME !== undefined && env.GAME !== game) {
    throw new Error(`GAME conflicts with deployment manifest (${env.GAME} != ${game})`);
  }
  if (env.BUILD !== undefined && env.BUILD !== buildId) {
    throw new Error(`BUILD conflicts with deployment manifest (${env.BUILD} != ${buildId})`);
  }
  return { game, buildId };
}

module.exports = { loadDeploymentManifest, browserOptionsFromEnvironment };
