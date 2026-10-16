# TUI WSS ERP connection

This is the first bounded implementation of #16: the TUI ERP source can connect
with `wss://`. Native relay WSS, native WebTransport, server changes and the
remaining epic acceptance criteria are outside this change.

## Trust and API contract

- `SocketSource::connect` retains its API and selects TLS for `wss://`, plain
  WebSocket for `ws://`, and newline-delimited JSON for `tcp://`.
- The default TLS trust store is the Mozilla root set shipped by `webpki-roots`.
  Certificate chain, validity and DNS/IP hostname checks are mandatory.
- `SocketSource::connect_with_options(..., &SocketOptions)` optionally takes
  `ca_file`, a PEM file containing the explicitly trusted certificates. It
  **replaces**, rather than expands, the public root set for this connection.
  An empty/invalid file fails. Hostname/chain checks still apply. This is useful
  for a private development CA; it is not certificate-verification bypass.
- CLI: `orr_tui --connect wss://localhost:7777 --ca-file ./dev-ca.pem --token ...`.
  `--ca-file` is rejected for non-WSS and other source modes. ERP rejects
  `--insecure`; the existing relay C-ABI option does not configure ERP TLS.
- The explicit token is sent in ERP `auth` after TLS and HTTP upgrade.
  Existing query-token endpoints remain compatible. Avoid query tokens and
  command-line tokens where process lists/history expose secrets. Diagnostic
  endpoint labels exclude userinfo, path, query and fragment. Handshake and
  authentication failures do not echo peer-supplied responses.
- The default 10-second `handshake_timeout` bounds TCP connection attempts,
  TLS and WebSocket upgrade together, including partial record/header reads.
  OS DNS resolution is synchronous and outside this I/O deadline guarantee.
  Each `recv(timeout)` bounds underlying socket reads; recoverable timeout and
  Interrupted behavior preserves the same WebSocket and partial frame.
  Sends use a 30-second deadline. Existing ERP response/schema deadlines remain
  30/10 seconds. Close is reported as a closed connection, not a timeout.
- This changes transport, not capabilities: ERP authentication/authorization
  still applies. The viewer still uses only the view-stream format, with its
  existing input/timeline control methods; it does not link simulation code.

## Dependency boundary

The blocking client uses rustls with ring/std/TLS 1.2 and 1.3, plus webpki-roots.
There is no native TLS dependency, async runtime, QUIC or HTTP/3 stack in the
normal TUI dependency tree. `rcgen` is test-only. All versions already exist in
this workspace lockfile; only TUI dependency edges are added.

## Focused verification

```
cargo test -p orr_tui --release --test wss --lib --test deps
cargo test -p orr_tui --release
cargo test -p orr_tui --no-default-features --release --lib --test wss --test deps
cargo clippy -p orr_tui --all-targets -- -D warnings
```

`tests/wss.rs` terminates real loopback TLS in front of unchanged ERP hosts.
It checks authenticated 2D headless output against the existing Rust-bridge
checksum, Yard3D schema/frame bytes against plain WS, rejection of an untrusted
CA and a trusted certificate for the wrong hostname, query-token redaction,
handshake/read deadlines, and close. Source unit tests retain deterministic
Interrupted/partial-frame regressions. Existing WS/TCP tests and `tests/deps.rs`
are kept intact. Linux checks do not establish other-platform support or native
relay/WebTransport interoperability.
