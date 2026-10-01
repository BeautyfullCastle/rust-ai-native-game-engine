// Browser transport of the Orrery relay client (bundled into the wasm package
// by wasm-bindgen as a module snippet).
//
// One connection, two channels, the same wire format as the native backends
// (crates/orr_net/src/lib.rs):
//   WebTransport  reliable   = the first bidirectional stream, frames `u32 LE length | u8 tag | payload`,
//                              the first frame is the hello (tag 2, "ORRN\x01");
//                 unreliable = datagrams, raw payload.
//   WebSocket     binary messages `tag | payload` (tag 0 reliable, 1 unreliable), sub-protocol "orrery.1".
//
// opts: {
//   mode: "auto" | "webtransport" | "websocket"   (auto: WebTransport first, WebSocket when it fails or is missing)
//   wtUrl:  "https://host:port/"                  WebTransport endpoint
//   certHash: "hex sha-256"                       serverCertificateHashes (dev self-signed certs; leave empty for a CA cert)
//   wsUrl:  "ws://host:port/"                     WebSocket endpoint
//   wtTimeoutMs: 4000                             how long WebTransport may take before falling back
// }
// onEvent(kind, a, data): kind 0 connected (a = largest unreliable payload: the datagram size, 1200 on WebSocket),
//   kind 1 disconnected, kind 2 message (a = 0 reliable / 1 unreliable, data = Uint8Array).

const HELLO = new Uint8Array([0x4f, 0x52, 0x52, 0x4e, 0x01]);
const TAG_RELIABLE = 0;
const TAG_UNRELIABLE = 1;
const TAG_HELLO = 2;

function frame(tag, payload) {
  const b = new Uint8Array(5 + payload.length);
  new DataView(b.buffer).setUint32(0, payload.length + 1, true);
  b[4] = tag;
  b.set(payload, 5);
  return b;
}

export class Transport {
  constructor(opts, onEvent) {
    this.opts = opts || {};
    this.onEvent = onEvent;
    this._kind = "";
    this._error = "";
    this.closed = false;
    this.up = false;
    this.wt = null;
    this.ws = null;
    this.maxDatagramSize = 0;
    this.start().catch((e) => this.fail(String((e && e.message) || e)));
  }

  get kind() { return this._kind; }
  get error() { return this._error; }

  fail(msg) {
    this._error = msg;
    this.down();
  }

  down() {
    if (this.up) {
      this.up = false;
      this.onEvent(1, 0, undefined);
    }
    this.up = false;
  }

  async start() {
    const mode = this.opts.mode || "auto";
    const wantWt = mode !== "websocket" && this.opts.wtUrl && typeof WebTransport !== "undefined";
    if (wantWt) {
      try {
        await this.startWebTransport();
        return;
      } catch (e) {
        this._error = "webtransport: " + String((e && e.message) || e);
        if (mode === "webtransport") throw new Error(this._error);
        try { this.wt && this.wt.close(); } catch (_) { /* ignore */ }
        this.wt = null;
      }
    } else if (mode === "webtransport") {
      throw new Error("WebTransport is not available or no wtUrl given");
    }
    if (!this.opts.wsUrl) throw new Error(this._error || "no transport available");
    await this.startWebSocket();
  }

  async startWebTransport() {
    const o = this.opts;
    const init = {};
    if (o.certHash) {
      const bytes = new Uint8Array(o.certHash.match(/../g).map((h) => parseInt(h, 16)));
      init.serverCertificateHashes = [{ algorithm: "sha-256", value: bytes }];
    }
    const wt = new WebTransport(o.wtUrl, init);
    this.wt = wt;
    const timeout = new Promise((_, rej) => setTimeout(() => rej(new Error("connect timeout")), o.wtTimeoutMs || 4000));
    await Promise.race([wt.ready, timeout]);
    const bidi = await wt.createBidirectionalStream();
    this.writer = bidi.writable.getWriter();
    await this.writer.write(frame(TAG_HELLO, HELLO));
    this.dgWriter = wt.datagrams.writable.getWriter();
    this.maxDatagramSize = wt.datagrams.maxDatagramSize | 0;
    this._kind = "webtransport";
    this._error = "";
    this.up = true;
    this.onEvent(0, this.maxDatagramSize, undefined);
    this.readStream(bidi.readable.getReader()).catch(() => this.down());
    this.readDatagrams(wt.datagrams.readable.getReader()).catch(() => {});
    wt.closed.then(() => this.down(), (e) => { this._error = "closed: " + String((e && e.message) || e); this.down(); });
  }

  async readStream(reader) {
    let buf = new Uint8Array(0);
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      if (buf.length === 0) buf = value; else { const nb = new Uint8Array(buf.length + value.length); nb.set(buf); nb.set(value, buf.length); buf = nb; }
      while (buf.length >= 5) {
        const len = new DataView(buf.buffer, buf.byteOffset).getUint32(0, true);
        if (buf.length < 4 + len) break;
        const tag = buf[4];
        const payload = buf.slice(5, 4 + len);
        buf = buf.slice(4 + len);
        if (tag === TAG_RELIABLE || tag === TAG_UNRELIABLE) this.onEvent(2, tag, payload);
      }
    }
    this.down();
  }

  async readDatagrams(reader) {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      this.onEvent(2, TAG_UNRELIABLE, value);
    }
  }

  startWebSocket() {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(this.opts.wsUrl, "orrery.1");
      ws.binaryType = "arraybuffer";
      this.ws = ws;
      let opened = false;
      ws.onopen = () => {
        opened = true;
        this._kind = "websocket";
        this.up = true;
        // Unreliable messages ride the ordered stream (the server drops them when its queue is long).
        this.onEvent(0, 1200, undefined);
        resolve();
      };
      ws.onmessage = (ev) => {
        const b = new Uint8Array(ev.data);
        if (b.length >= 1 && (b[0] === TAG_RELIABLE || b[0] === TAG_UNRELIABLE)) this.onEvent(2, b[0], b.slice(1));
      };
      ws.onerror = () => { if (!opened) reject(new Error("websocket error")); };
      ws.onclose = () => { if (opened) this.down(); else reject(new Error("websocket closed")); };
    });
  }

  send(channel, data) {
    if (!this.up) return;
    if (this.wt) {
      if (channel === TAG_UNRELIABLE) {
        // Datagrams are best effort: drop when the browser's queue is full.
        if (this.dgWriter.desiredSize !== null && this.dgWriter.desiredSize <= 0) return;
        this.dgWriter.write(data.slice()).catch(() => {});
      } else {
        this.writer.write(frame(TAG_RELIABLE, data)).catch(() => this.down());
      }
    } else if (this.ws && this.ws.readyState === 1) {
      if (channel === TAG_UNRELIABLE && this.ws.bufferedAmount > 64 * 1024) return;
      const b = new Uint8Array(1 + data.length);
      b[0] = channel === TAG_UNRELIABLE ? TAG_UNRELIABLE : TAG_RELIABLE;
      b.set(data, 1);
      this.ws.send(b);
    }
  }

  close() {
    this.closed = true;
    try { if (this.wt) this.wt.close({ closeCode: 0, reason: "bye" }); } catch (_) { /* ignore */ }
    try { if (this.ws) this.ws.close(); } catch (_) { /* ignore */ }
  }
}
