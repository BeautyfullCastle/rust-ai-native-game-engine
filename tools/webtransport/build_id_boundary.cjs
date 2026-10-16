"use strict";

// Self-contained so the same assertions run inside Chromium with page.evaluate
// and inside Node against the generated JS/WASM package. Only the socket is a
// fake: constructors, option parsing, client tick and Hello encoding are real.
async function checkBuildIdBoundary({ moduleUrl = "/pkg/orr_web.js", moduleBytes } = {}) {
  const { default: init, WebClient, PhysClient } = await import(moduleUrl);
  await init(moduleBytes ? { module_or_path: moduleBytes } : undefined);
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const originalSocket = globalThis.WebSocket;
  const sockets = [];
  class TestSocket {
    constructor() {
      this.readyState = 1;
      this.bufferedAmount = 0;
      this.messages = [];
      sockets.push(this);
      queueMicrotask(() => this.onopen());
    }
    send(bytes) { this.messages.push(bytes.slice()); }
    close() { this.readyState = 3; if (this.onclose) this.onclose(); }
  }
  // Independent integer reference for build_hash_of(id, 0), SplitMix64's
  // finalizer. No Number conversion is allowed at this reference boundary.
  const hash = (id) => {
    let x = BigInt(id);
    x = BigInt.asUintN(64, (x ^ (x >> 30n)) * 0xbf58476d1ce4e5b9n);
    x = BigInt.asUintN(64, (x ^ (x >> 27n)) * 0x94d049bb133111ebn);
    return x ^ (x >> 31n);
  };
  const good = [
    0, 1, 42, Number.MAX_SAFE_INTEGER,
    "0", "42", "9007199254740991", "9007199254740992", "9007199254740993",
    "18446744073709551615", "0xffffffffffffffff", "0x20000000000001", " 0X20000000000001 ",
  ];
  const bad = [
    true, false, {}, [], 1n, -1, 0.5, 1.5, Number.MAX_SAFE_INTEGER + 1,
    Number("9007199254740993"), Number("18446744073709551615"), NaN, Infinity, -Infinity,
    "", " ", "0x", "-1", "+1", "0x-1", "0x+1", "1.5", "1e3", "0xGG", "NaN",
    "18446744073709551616", "0x10000000000000000",
  ];
  let accepted = 0;
  let rejected = 0;
  globalThis.WebSocket = TestSocket;
  try {
    // The single expected format input for both Node and Chromium. Intentional
    // ORRF changes update this version, never opaque per-constructor hash values.
    const expectedFrameFormatVersion = 2n;
    for (const [Client, gameBuildId] of [[WebClient, 0x0a2e4a000001n], [PhysClient, 0x0a2e4a000002n]]) {
      const hello = async (build_id) => {
        const client = new Client({ build_id, mode: "websocket", wsUrl: "ws://test.invalid" });
        try {
          await Promise.resolve(); // deliver the fake socket's open event
          client.tick(0);
          const socket = sockets.at(-1);
          const bytes = socket.messages.find((b) => b[0] === 0 && b[9] === 1);
          assert(bytes, `${Client.name}: no reliable Hello was encoded`);
          assert(String.fromCharCode(...bytes.slice(1, 5)) === "ORRN", "bad Hello magic");
          accepted++;
          return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getBigUint64(10, true);
        } finally {
          client.close();
          client.free();
        }
      };
      // Independently derive frame_build_id(game) then build_hash_of(id, 0).
      // Undefined/null must retain the right game's default, not just agree.
      const formatId = 0x4f52524600000000n | expectedFrameFormatVersion;
      const frameBuildId = hash(gameBuildId ^ hash(formatId)) || 1n;
      const expectedDefaultHash = hash(frameBuildId);
      assert(await hello(undefined) === expectedDefaultHash, `${Client.name}: undefined default build id changed`);
      assert(await hello(null) === expectedDefaultHash, `${Client.name}: null default build id changed`);
      for (const value of good) {
        assert(await hello(value) === hash(value), `${Client.name}: build_id ${String(value)} lost precision`);
      }
      for (const value of bad) {
        const before = sockets.length;
        let error;
        try {
          const client = new Client({ build_id: value, mode: "websocket", wsUrl: "ws://test.invalid" });
          // A mutant parser must still clean up before the assertion fails.
          await Promise.resolve();
          client.close();
          client.free();
        } catch (e) { error = String(e); }
        assert(error && error.includes("build_id"), `${Client.name}: accepted invalid build_id ${String(value)}`);
        assert(sockets.length === before, `${Client.name}: opened a socket before rejecting build_id`);
        rejected++;
      }
    }
  } finally {
    if (originalSocket === undefined) delete globalThis.WebSocket;
    else globalThis.WebSocket = originalSocket;
  }
  return { constructors: 2, accepted, rejected };
}

module.exports = { checkBuildIdBoundary };
