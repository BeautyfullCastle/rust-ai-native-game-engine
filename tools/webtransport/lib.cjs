"use strict";
// Shared helpers of the browser tests: find Playwright, launch headless
// Chromium, start a child process that prints a READY line, serve static files.

const { spawn, execSync } = require("child_process");
const fs = require("fs");
const http = require("http");
const path = require("path");

function loadPlaywright() {
  const tries = [() => require("playwright"), () => require(path.join(execSync("npm root -g").toString().trim(), "playwright"))];
  for (const t of tries) {
    try { return t(); } catch (_) { /* next */ }
  }
  return null;
}

/** Requirement flags use the same exact `=1` convention as the Rust harness. */
function requiresBrowser(env = process.env) {
  return env.ORR_REQUIRE_BROWSER === "1" || env.ORR_REQUIRE_WEBGPU === "1";
}

function unavailableExitCode(env = process.env) {
  return requiresBrowser(env) ? 1 : 2;
}

/** Chromium launch options. CHROMIUM=path overrides the executable; CHROMIUM_FLAGS adds flags (space separated). */
function chromiumOptions(extraArgs = []) {
  const args = [...extraArgs, ...(process.env.CHROMIUM_FLAGS ? process.env.CHROMIUM_FLAGS.split(/\s+/).filter(Boolean) : [])];
  const opts = { headless: true, args };
  if (process.env.CHROMIUM) opts.executablePath = process.env.CHROMIUM;
  return opts;
}

/** Spawns `cmd`, resolves with {proc, ready} once a stdout line starting with `READY ` appears (JSON after it). */
function spawnReady(cmd, args, { timeoutMs = 30000, env } = {}) {
  return new Promise((resolve, reject) => {
    const proc = spawn(cmd, args, { stdio: ["pipe", "pipe", "inherit"], env: { ...process.env, ...env } });
    let buf = "";
    const timer = setTimeout(() => { proc.kill(); reject(new Error(`${cmd}: no READY line within ${timeoutMs} ms`)); }, timeoutMs);
    proc.on("exit", (c) => { clearTimeout(timer); reject(new Error(`${cmd} exited early (${c})`)); });
    proc.stdout.on("data", (d) => {
      buf += d.toString();
      process.stdout.write(d);
      const m = buf.match(/^READY (.*)$/m);
      if (m) { clearTimeout(timer); proc.removeAllListeners("exit"); resolve({ proc, ready: JSON.parse(m[1]) }); }
    });
  });
}

/** Serves `dir` on 127.0.0.1 (random port), with the given extra headers. */
function serveStatic(dir, headers = {}) {
  const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };
  const server = http.createServer((req, res) => {
    const p = path.normalize(decodeURIComponent(req.url.split("?")[0])).replace(/^(\.\.[/\\])+/, "");
    const file = path.join(dir, p.endsWith("/") ? p + "index.html" : p);
    fs.readFile(file, (err, data) => {
      if (err) { res.writeHead(404); return res.end("not found"); }
      res.writeHead(200, { "content-type": types[path.extname(file)] || "application/octet-stream", ...headers });
      res.end(data);
    });
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve({ server, port: server.address().port })));
}

function repoRoot() {
  return path.resolve(__dirname, "..", "..");
}

module.exports = { loadPlaywright, chromiumOptions, spawnReady, serveStatic, repoRoot, requiresBrowser, unavailableExitCode };
