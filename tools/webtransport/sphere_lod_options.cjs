"use strict";
// A requested hardware run is distinct from the default SwiftShader contract.
function sphereOptions(backend, env = process.env, platform = process.platform) {
  if (!["webgl", "webgpu"].includes(backend)) throw new Error("SPHERE_LOD_BACKEND must be webgpu or webgl");
  const adapter = env.ORR_SPHERE_WEBGPU_ADAPTER || "swiftshader";
  if (!["swiftshader", "hardware"].includes(adapter)) throw new Error("invalid ORR_SPHERE_WEBGPU_ADAPTER");
  const headed = env.ORR_SPHERE_HEADED || "0";
  if (!["0", "1"].includes(headed)) throw new Error("ORR_SPHERE_HEADED must be 0 or 1");
  const extra = (env.CHROMIUM_FLAGS || "").split(/\s+/).filter(Boolean);
  for (const flag of extra) {
    if (/^--(?:no-sandbox|disable-.*sandbox|disable-web-security|disable-site-isolation-trials|ignore-certificate-errors|allow-insecure-localhost)(?:=|$)/.test(flag)) throw new Error("security weakening Chromium flag is forbidden");
    if (/^--(?:use-webgpu-adapter|use-angle|use-gl|use-vulkan|headless|disable-gpu)(?:=|$)/.test(flag)) throw new Error("select backend/headed mode through explicit sphere options");
    if (!/^--(?:enable-logging(?:=stderr)?|log-level=[0-3]|v=\d+)$/.test(flag)) throw new Error("sphere CHROMIUM_FLAGS accepts diagnostic logging flags only");
  }
  let args;
  if (backend === "webgl") args = ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"];
  else if (adapter === "hardware") args = platform === "win32" ? ["--use-angle=d3d11"] : [];
  else args = platform === "win32"
    ? ["--enable-unsafe-webgpu", "--use-webgpu-adapter=swiftshader", "--use-gpu-in-tests"]
    : ["--enable-unsafe-webgpu", "--enable-unsafe-swiftshader", "--use-webgpu-adapter=swiftshader", "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--use-angle=swiftshader"];
  return { args, headless: headed !== "1", adapter: backend === "webgpu" ? adapter : "swiftshader" };
}
module.exports = { sphereOptions };
