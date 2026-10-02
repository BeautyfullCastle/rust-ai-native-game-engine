# WebTransport trial and the browser client

Status: implemented and verified in headless Chromium 141 (Linux). Firefox and
Safari are **not** verified, see section 7 for the exact checklist.
This is the result of the trial that `docs/design-v1.md` §11 asks for ("do a
trial implementation before the real one") and of the real browser path of §6.1.
`docs/progress.md` and `docs/design-v1.md` are not edited here; fold the
conclusions in when you review.

## 1. Summary

| Question | Answer |
|---|---|
| Crate | `h3` 0.0.8 + `h3-webtransport` 0.1.2 + `h3-quinn` 0.0.10 (all on the workspace's `quinn` 0.11.12), not `wtransport` |
| Can QUIC `orrery/1` and WebTransport `h3` share one UDP port? | **Yes.** One `quinn::Endpoint`, ALPN list `[orrery/1, h3]`, dispatch after the handshake on the negotiated ALPN. Tested. |
| Certificates | Dev: self-signed ECDSA P-256, valid 13 days (`QuicServerTls::SelfSignedWebTransport`), SHA-256 printed for `serverCertificateHashes`. Production / Safari: PEM files (`--tls-cert/--tls-key`) with a CA-signed certificate. |
| Chromium needs flags? | **None.** Playwright's Chromium 141, headless, page on `http://localhost`, `serverCertificateHashes`. |
| Latency (loopback, Chromium, 200 pings) | WebTransport stream p50 0.9 ms / p95 1.2 ms, datagrams p50 0.9 ms / p95 1.5 ms, WebSocket p50 0.7 ms / p95 1.2 ms. Connect: WebTransport 17-41 ms, WebSocket 4-6 ms. |
| Browser determinism proof | Chromium running the wasm client plays 600+ ticks with a native client: 20 of 20 verified checkpoints equal, 0 desyncs, 136 rollbacks in the browser, over WebTransport, over WebSocket fallback and over `wss://`. |
| Safari risk | Real: Safari has no `serverCertificateHashes` (needs a CA certificate even in dev) and its WebTransport draft may not match `h3`'s (hyperium/h3 #347). **Untested.** |

## 2. Crate choice

* `wtransport` 0.7 is the more polished API and speaks the newest settings ids,
  but it owns its `quinn::Endpoint` (it builds one from its own config with
  ALPN `h3`) and has no hook to hand it a connection that arrived on a shared
  endpoint. With it, sharing a port with `orrery/1` is impossible, so the
  native QUIC clients would need a second port. It is kept as a **dev
  dependency**: the tests in `crates/orr_net/tests/webtransport.rs` use the
  `wtransport` *client* against our server, which is also an interop check
  between two independent implementations.
* `h3` + `h3-webtransport` + `h3-quinn` work on any `quinn::Connection`. Our
  `quic::accept_loop` already owns the endpoint, so after the handshake we look
  at the ALPN and give `h3` connections to `crates/orr_net/src/wt.rs`. The
  crates are 0.0.x / 0.1.x (hyperium says "not stable"), so the three versions
  are pinned in `crates/orr_net/Cargo.toml`.
* `h3-webtransport` speaks the draft-02 style (setting `0x2b603742`,
  `sec-webtransport-http3-draft: draft02`). Chromium 141 and the `wtransport`
  client accept it. Whether current Firefox and Safari accept it is open.
  **Fallback if they do not:** write the WebTransport CONNECT handshake and
  session framing ourselves on `h3`'s lower layers, or move the browser path to
  `wtransport` on a second UDP port (the `Link` code above it does not change).

## 3. One UDP port for `orrery/1` and `h3`

`Endpoint::listen_quic_with(bind, tls, cfg, webtransport = true)`:

1. The rustls config of the quinn endpoint lists both ALPNs (`orrery/1`, `h3`).
2. `quic::server_conn` awaits the QUIC handshake, then `wt::is_h3` reads
   `HandshakeData.protocol`. `h3` goes to `wt::accept` (HTTP/3 SETTINGS, the
   extended CONNECT `:protocol = webtransport`, then the first bidirectional
   stream must start with the hello frame), anything else takes the existing
   native path unchanged.
3. Everything after that is the same as for native clients: the connection gets
   an `orr_net::ConnId`, `Connected`/`Message`/`Disconnected` events and stats.
   The relay (`orr_relay_net::NetEndpoint`, `orr_server::RelayServer`) cannot
   tell a browser from a native client.

Caveats:

* Transport parameters (stream limits) are per endpoint config and are sent
  before the ALPN is known, so with WebTransport on, native clients also get
  8 bidirectional / 8 unidirectional stream credits instead of 1 / 0. The
  native path still accepts exactly one stream; extra ones are never read.
  With WebTransport off nothing changes.
* An `h3` client against a server with WebTransport off fails the TLS handshake
  (no common ALPN). Tested (`h3_is_refused_when_webtransport_is_off`).

Tests: `crates/orr_net/tests/webtransport.rs`
(`webtransport_and_quic_share_one_port` connects a native `orrery/1` client and
a `wtransport` client to the same port and moves stream and datagram messages
both ways; plus refused-without-hello, h3-refused-when-off, and `wss://`).

## 4. Wire format and channels

Same messages as every transport (`orr_proto`); only the framing carrier differs
(`crates/orr_net/src/lib.rs` has the full table).

| | Reliable | Unreliable |
|---|---|---|
| WebTransport | first bidirectional stream opened by the browser: hello frame (`ORRN\x01`, tag 2), then frames `u32 LE length | tag | payload` | WebTransport datagrams, raw payload |
| WebSocket | binary message `tag | payload` (tag 0) | same stream, tag 1 |

* Chrome's `datagrams.maxDatagramSize` is 1024. `WebLink` (and the native
  `NetLink`) send a larger unreliable message on the reliable channel; the
  server side computes the datagram limit as quinn's limit minus the 2 bytes of
  WebTransport framing.
* A browser cannot offer the WebSocket sub-protocol `orrery/1` (`/` is not a
  token character). The server accepts `orrery/1` (native) and `orrery.1`
  (browser).

## 5. Certificates

* **Dev, Chrome/Firefox:** `QuicServerTls::SelfSignedWebTransport` (used by
  `orr_server --webtransport` without `--tls-cert`) generates ECDSA P-256 valid
  from one hour ago for 13 days. The server prints the SHA-256 (hex); the page
  passes it as `serverCertificateHashes`. Verified: a certificate valid for
  decades (`QuicServerTls::SelfSigned`) is **refused** by Chromium with that
  hash ("Opening handshake failed"), so the 14-day rule is real. Restart the
  dev server before day 13.
* **Production, Safari, any browser without `serverCertificateHashes`:** a
  CA-signed certificate in PEM files: `orr_server --tls-cert cert.pem --tls-key key.pem`
  (chain first in the cert file; PKCS#8, PKCS#1 or SEC1 key). Local CA for
  development: `mkcert -ecdsa -install; mkcert -ecdsa -cert-file cert.pem -key-file key.pem localhost 127.0.0.1 ::1 <lan-ip> <host>.local`
  (iOS also needs the mkcert root installed and enabled under Settings > General >
  About > Certificate Trust Settings).
* Measured on Chromium 141 with a PEM certificate and no hash:
  untrusted certificate: refused. `--ignore-certificate-errors` alone: still
  refused (it does not apply to QUIC). `--ignore-certificate-errors-spki-list=<base64 sha256 of the SPKI>`
  with `--origin-to-force-quic-on=127.0.0.1:<port>`: connects. PEM certificate
  plus `serverCertificateHashes`: connects.
* `wss://`: `orr_server --wss-bind ADDR` serves TLS WebSocket with the same
  certificate (an `https://` page must use `wss://`, no mixed content).

## 6. Browser results (headless Chromium 141, Linux, loopback)

Run: `cargo build -p orr_net --release --example wt_echo` then
`node tools/webtransport/trial.cjs` (Playwright from `NODE_PATH` or the global
npm root; `CHROMIUM=/path` and `CHROMIUM_FLAGS` override; no flags needed).

```
webtransport: connected, connect 16.9 ms, maxDatagramSize 1024
  stream   200/200  min 0.6  p50 0.9  p95 1.2  max 13.3 ms
  datagram 200/200  min 0.6  p50 0.9  p95 1.5  max 5.1 ms
websocket:    connected, connect 5.8 ms
  stream   200/200  min 0.4  p50 0.7  p95 1.2  max 4.0 ms
```

Another run: connect 41 ms (cold), stream p50 0.7, datagram p50 0.8. These are
loopback round trips through the browser, the echo server and h3; they say the
stack adds about 1 ms, not what a real network does. In the proof test below the browser's measured round trip
to the relay was 83 ms with 2 x 35 ms of injected delay (native client: 80 ms).

Needed flags: none. If a Chromium refuses localhost QUIC, try
`--origin-to-force-quic-on=127.0.0.1:<port>` and, for a non-hash certificate,
`--ignore-certificate-errors-spki-list=...` (the page must be served from
`http://localhost` or `https://`, WebTransport needs a secure context).

### Determinism proof (the real client)

`crates/orr_server/tests/browser_e2e.rs`: an in-process relay server (QUIC +
WebTransport on one port, plus WebSocket, 35 ms +-8 ms injected delay each way),
a native Rust bot client and the wasm client in Chromium share a 2-player
arena room (build id 1) and play with scripted bot inputs until the browser has
600 verified ticks. Output of one run:

```
browser: transport webtransport slot 1 rollbacks 136 (resim 681 ticks, depth 6) verified 600 head 604 desyncs 0 rtt 83299 us delay 2
native : slot 0 | rtt 79 ms | delay 2 | rollbacks 156 (15.0/s) | verified 623 (head 627) | desyncs 0
compared 20 verified checkpoints (browser has 20, native 20): all equal
```

The same assertions pass for `browser_falls_back_to_websocket` (server without
WebTransport; the page starts in `auto` mode, WebTransport fails, WebSocket
takes over; transport reported `websocket`) and `browser_joins_over_secure_websocket`
(`wss://`, Chromium told to accept the self-signed certificate). The checksums
of the WebTransport and the WebSocket runs I compared were identical tick for tick
(same room seed, same bots; not guaranteed in general, because the server repeats
an input that arrives late), so the transport does not change the simulation.

Run it:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # the wasm-bindgen version in Cargo.lock
tools/build_web.sh                                           # crates/orr_web/web/pkg
cargo test -p orr_server --release --test browser_e2e -- --nocapture --test-threads=1
```

Needs Node.js and Playwright (`npm i -g playwright`, a Chromium in
`PLAYWRIGHT_BROWSERS_PATH`, or `CHROMIUM=/path/to/chrome`). Without the pieces
each test prints `SKIP` and passes; with `ORR_REQUIRE_BROWSER=1` a missing
piece fails the test. The test is **not** in CI (browser, Node and
wasm-bindgen-cli are heavy); CI builds the wasm crate (section 8).

## 7. Firefox and Safari: what must be checked by hand

Everything below needs the ready-to-run pages in `tools/webtransport/`.

**Server (any machine; LAN address for other devices):**

```sh
cargo run -p orr_net --release --example wt_echo -- --bind 0.0.0.0 --udp 4433 --ws 4434
# prints: READY {"udp_port":4433,"ws_port":4434,"hash":"<sha256 hex>"}
```

For Safari or any CA-certificate test add `--cert cert.pem --key key.pem` (see
section 5; the printed hash is then `null`). For the real game:
`orr_server --transport quic --webtransport --ws-bind 0.0.0.0:4434 [--tls-cert .. --tls-key ..]`.

**Firefox on the Windows machine (hash mode):**

1. Start `wt_echo` as above (`--bind 127.0.0.1`), note the hash.
2. Serve the page: `python -m http.server 8000 --directory tools\webtransport`.
3. Open `http://localhost:8000/trial.html?host=127.0.0.1&port=4433&wsport=4434&hash=<hash>&auto=1`.
4. Record the JSON shown: `webtransport.connected`, `stream`/`datagram` counts,
   `max_datagram_size`, `websocket.connected`. Pass = WebTransport connected with
   200/200 stream and (nearly) all datagrams.
5. If it fails with the hash: check whether this Firefox has
   `serverCertificateHashes` (`about:support`, version; the result says "is not
   defined" for a missing API) and whether it rejects the draft: run the Rust
   server console and the browser console for the error text. Then retry with
   an mkcert certificate (no hash) per section 5. Report which of the three
   failed: API missing, certificate refused, or session handshake refused.
6. Then run the real client: build `tools/build_web.sh`, serve
   `crates\orr_web\web`, start `orr_server --webtransport --ws-bind ...`, open
   `index.html?auto=1&bot=1&wt=127.0.0.1:4433&hash=<hash>&ws=127.0.0.1:4434&mode=webtransport`
   (omit `build` to use the arena default shared with `--game arena`; built-in IDs now
   include the ORRF format via `orr_sim::frame_build_id`, see [compatibility](frame-compatibility.md)),
   watch `desyncs 0` and the checksum line against a native `orr_sample --headless --bot --connect`.

**Safari (Mac, then iPhone/iPad):** Safari has no `serverCertificateHashes`, so
a trusted certificate is mandatory.

1. `mkcert -ecdsa -install`, then a certificate for the Mac's LAN name/IP (section 5).
   For the iPhone, AirDrop `rootCA.pem` (`mkcert -CAROOT`), install the profile,
   enable it in Certificate Trust Settings.
2. `wt_echo --bind 0.0.0.0 --udp 4433 --ws 4434 --cert cert.pem --key key.pem`.
3. Serve the page over HTTPS (secure context on a LAN name):
   `npx http-server tools/webtransport -S -C cert.pem -K key.pem -p 8443`.
4. Mac Safari (26.4 or later): `https://<host>:8443/trial.html?host=<host>&port=4433&wsport=4434&auto=1`
   (no hash). The WebSocket part uses `ws://`, which an `https://` page cannot
   open (mixed content): for that run the server with `--wss-bind` and use the
   index page with `ws=wss://<host>:<port>/` instead, or test WebSocket from
   an `http://localhost` page.
5. Record: `webtransport.api` (is `WebTransport` defined), `connected`, error
   text, datagram support, `max_datagram_size`. The known failure shape is
   hyperium/h3 #347 (Safari's older draft): the browser reports a failed
   session or the server logs nothing for the CONNECT. In that case run
   `wtransport` on a second port as the browser endpoint (section 2).
6. iPhone: same URL; also try a cellular network (UDP/443 vs blocked UDP) and
   confirm `auto` mode falls back to WebSocket within the 4 s timeout.

## 8. What is in the repo

| Piece | Where |
|---|---|
| WebTransport backend, ALPN dispatch | `crates/orr_net/src/wt.rs`, `quic.rs` (`server_conn`), `endpoint.rs` (`listen_quic_with`, `add_ws_listener`, `add_wss_listener`), `tls.rs` (`Identity`, `SelfSignedWebTransport`) |
| Relay glue | `crates/orr_relay_net/src/connect.rs` (`ListenOptions::{webtransport, ws_bind, wss_bind}`), `orr_server` flags `--webtransport --ws-bind --wss-bind` |
| Browser client | `crates/orr_web` (`WebLink`, the bots, the report, the draw lists, `js/transport.js`, `web/worker.js`, `web/index.html`), `crates/orr_web_gpu` (the GPU view), `tools/build_web.sh` |
| Trial tools | `crates/orr_net/examples/wt_echo.rs`, `tools/webtransport/{trial.html,trial.cjs,lib.cjs,browser_e2e.cjs}` |
| Tests | `crates/orr_net/tests/webtransport.rs`, `ws_malformed.rs`, `crates/orr_server/tests/browser_e2e.rs`, unit tests in `orr_web` |
| CI | `.github/workflows/determinism.yml`: job `wasm32-browser` builds the sim crates and `orr_web` for `wasm32-unknown-unknown` and runs clippy `-D warnings` on `orr_web`; `orr_net` and `orr_web` join the `-D warnings` clippy line |

The sim crates (`orr_fp`, `orr_ecs`, `orr_sim`, `orr_session`, `orr_proto`,
`orr_testgame`, `orr_physics`) build unchanged for `wasm32-unknown-unknown`.
`orr_web` compiles its browser glue only for wasm32 (so native
`cargo test --workspace` stays light) and has no float arithmetic of its own:
the page passes microseconds, the client returns world units as integers, and the
canvas code in `index.html` does the floating-point drawing.

Choice worth knowing: the browser transport is a small JS module bundled by
wasm-bindgen (`crates/orr_web/js/transport.js`) instead of `web-sys` bindings.
`web-sys`'s WebTransport types need `--cfg web_sys_unstable_apis` and manual
stream plumbing; the JS module is about 150 lines, easy to read next to the wire
format, and the Rust side only sees `LinkPort` events.

## 9. Gaps and next steps

* Firefox and Safari are untested (section 7). The biggest unknown is the draft
  that `h3-webtransport` speaks versus Safari 26.4 and Firefox.
* ~~The sim loop runs on `setInterval(…, 2)` in the page~~ Done (M6 step 4): see section 10. The relay client,
  the simulation and the transport run in a module Web Worker (`web/worker.js`).
* The page needs the certificate hash and server address from the URL. A small
  config endpoint (or the matchmaking service) should hand them out; the hash
  changes every restart of a dev server and every 13 days.
* ~~The physics game has no browser view~~ Done (M6 step 4): `PhysGame` moved to the sim-only crate `orr_games` and
  runs in the browser, drawn on a 2D canvas, WebGL2 or WebGPU (section 10). Still missing: audio, touch input, an
  input mapping beyond arrow keys, Q/E and space, a camera for the 3D yard (`Yard3D` builds for wasm but has no
  browser view; the 3D shader has not been run through a browser's WGSL validator).
* `h3`/`h3-webtransport` are 0.x. Watch hyperium/h3 for the newer WebTransport
  draft, and keep the `wtransport` interop test as the early warning.
* Native WebTransport and `wss://` *clients* are not implemented (only servers and
  the browser); native clients keep using QUIC `orrery/1`.

## 10. M6 step 4: Web Worker, PhysGame and the GPU view

* **Web Worker.** `crates/orr_web/web/worker.js` owns the `WebClient`/`PhysClient`, its timer
  (`setInterval(step, 2)`, which a worker is not throttled on in a background tab the way a page is) and the
  transport (`js/transport.js` is imported by the wasm package, so it runs in the worker; WebTransport and
  WebSocket are available in dedicated workers). Why the transport is in the worker and not behind
  `postMessage`: network bytes then never touch the main thread, a janky page cannot delay inputs or acks, and
  the page needs only input events (in) and draw lists plus a report every 250 ms (out; `Int32Array`s are
  transferred, not copied). The page keeps `window.orr` as a view of the cached report for the end-to-end test.
  Headless Chromium has no hidden-tab state to test the throttling directly; the reasoning is the browsers'
  documented timer policy.
* **PhysGame in the browser.** `orr_web::PhysClient` (scene from the room's config blob, `bot_input` or keys) and an
  integer draw list (`render_phys`: shape, class, x, y, angle, size, speed). `browser_e2e` plays it against a
  native client over WebTransport and over WebSocket: 600 ticks, 20 of 20 checkpoints equal, 0 desyncs, rollbacks
  in the browser.
* **GPU view.** A separate package `crates/orr_web_gpu` (2.6 MB, loaded on demand) draws the same draw lists with
  `orr_render`'s 2D renderer through `orr_rhi::Wgpu::for_canvas` (async adapter and device creation, wgpu's
  WebGPU backend when the browser has an adapter, its WebGL2 backend otherwise, downlevel limits on GL). The page
  chooses WebGPU, then WebGL2, then a plain 2D canvas, or what `?render=2d|webgl|webgpu` asks for. 3D (`Renderer3D`)
  is not in the browser yet; the 2D renderer is what PhysGame needs, and the 3D path adds depth, MSAA resolve and
  a shadow pass that need their own browser testing.
* **Found by running in a browser:** the 2D shader called `fwidth` inside a branch on the shape kind. Native
  validators accept that, Chrome's WGSL validator (Tint) rejects it ("must only be called from uniform control
  flow"). The derivatives moved to the top of the function (`crates/orr_render/src/shader2d.wgsl`); the native GPU
  readback tests are unchanged.
* **Headless Chromium flags.** WebGL2 works without flags (SwiftShader). WebGPU needs
  `--enable-unsafe-webgpu --enable-unsafe-swiftshader --use-webgpu-adapter=swiftshader --enable-features=Vulkan --use-vulkan=swiftshader --use-angle=swiftshader`
  (the e2e test passes them). Without a WebGPU adapter the page falls back to WebGL2 by itself (`auto`).
* **Build.** `tools/build_web.sh` (profile `web`, optional `wasm-opt`, optional `WEB_SIMD=1`, `WEB_GPU=0` to skip
  the GPU package). Numbers and sizes: `docs/wasm-bench.md`.
* **Run the end-to-end test** (all views, both games):
  `ORR_REQUIRE_BROWSER=1 cargo test -p orr_server --release --test browser_e2e -- --nocapture --test-threads=1`.
  `ORR_REQUIRE_WEBGPU=1` makes a Chromium without WebGPU a failure instead of a skipped view check.
