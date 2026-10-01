// The simulation worker: the relay client (prediction, rollback, input delay, desync checks),
// the game's wasm simulation and the browser transport all live here, off the main thread.
//
// Why a worker: a background tab throttles main-thread timers to 1 Hz (and a busy main thread
// stalls the sim), so a client driven by `setInterval` on the page falls behind the room and
// stalls it. Worker timers are not throttled that way, and WebTransport / WebSocket are
// available in workers, so the network traffic never touches the main thread either: the page
// only sends input and receives draw lists (Int32Array, transferred) and reports.
//
// Messages page -> worker:
//   { type: "start", opts }          opts as for WebClient / PhysClient plus game: "arena" | "phys"
//   { type: "input", args }          client.set_input(...args)
//   { type: "bot", on }              client.set_bot(on)
//   { type: "leave" }                tell the server, close the transport
// Messages worker -> page:
//   { type: "ready" }                wasm loaded, client created
//   { type: "frame", data, box }     draw list (see render_arena / render_phys); box = [half_w, height] for phys
//   { type: "report", report, transportError, oversize, sim }   every 250 ms; sim = ms spent in client.tick
//   { type: "error", message }

import init, { WebClient, PhysClient } from "./pkg/orr_web.js";

let client = null;
let game = "arena";
let timer = null;
let lastFrame = 0;
let lastReport = 0;
let samples = [];
let totalTicks = 0;

const FRAME_MS = 16;
const REPORT_MS = 250;

function percentile(sorted, p) {
  return sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))] : 0;
}

function step() {
  const t0 = performance.now();
  try {
    client.tick(Math.floor(t0 * 1000));
  } catch (e) {
    postMessage({ type: "error", message: String((e && e.stack) || e) });
    clearInterval(timer);
    return;
  }
  const t1 = performance.now();
  samples.push(t1 - t0);
  totalTicks++;
  if (t1 - lastFrame >= FRAME_MS) {
    lastFrame = t1;
    const data = client.render();
    const msg = { type: "frame", data };
    if (game === "phys") msg.box = client.scene_box();
    postMessage(msg, [data.buffer]);
  }
  if (t1 - lastReport >= REPORT_MS) {
    lastReport = t1;
    samples.sort((a, b) => a - b);
    const sim = {
      calls: samples.length,
      mean: samples.reduce((a, b) => a + b, 0) / Math.max(1, samples.length),
      p50: percentile(samples, 0.5),
      p95: percentile(samples, 0.95),
      max: samples.length ? samples[samples.length - 1] : 0,
      total_calls: totalTicks,
    };
    samples = [];
    postMessage({
      type: "report",
      report: client.report(),
      transportError: client.transport_error(),
      oversize: client.oversize_to_reliable(),
      sim,
    });
  }
}

onmessage = async (e) => {
  const m = e.data;
  try {
    if (m.type === "start") {
      await init();
      game = m.opts.game === "phys" ? "phys" : "arena";
      client = game === "phys" ? new PhysClient(m.opts) : new WebClient(m.opts);
      postMessage({ type: "ready" });
      // Worker timers are clamped to 4 ms once nested; the client catches up on its own clock.
      timer = setInterval(step, 2);
    } else if (!client) {
      return;
    } else if (m.type === "input") {
      client.set_input(...m.args);
    } else if (m.type === "bot") {
      client.set_bot(!!m.on);
    } else if (m.type === "leave") {
      client.leave();
      // One more step flushes the leave message before the transport closes.
      step();
      setTimeout(() => client.close(), 100);
    }
  } catch (err) {
    postMessage({ type: "error", message: String((err && err.stack) || err) });
  }
};
