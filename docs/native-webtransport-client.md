# Native WebTransport client

`Endpoint::connect_wt("https://relay.example:443/relay?room=1", QuicTrust::WebPki, config)`
connects asynchronously using HTTP/3 WebTransport. Poll the normal transport events.
The existing QUIC server must enable `webtransport`; plain QUIC endpoints reject WT.
No browser, additional server, or alternate relay protocol is needed.

Relay clients use `TransportKind::Wt`. Samples accept `--transport wt --connect
https://relay.example:443/relay` and default to public WebPKI roots. Use `--ca-cert
ca.pem` for an explicit CA. Certificate chain, validity and URL hostname are always
verified. Fingerprint-only and insecure trust are rejected. WT does not support
`--server-name`: use the certificate hostname in the URL. WSS/QUIC retain their
existing trust and name-override policies.

Reliable messages use one framed bidirectional stream; unreliable messages use
actual WebTransport datagrams. Payloads above the current datagram limit use the
stream with their unreliable tag when `datagram_fallback` is enabled; otherwise
`send` returns `TooLarge`. The message-size ceiling still applies. WT outgoing
stream and datagram queues share the configured byte budget (empty messages cost
one byte), return `Backpressure` when full, and refuse sends after local close.
Incoming reliable events apply backpressure; excess datagrams may be dropped.
Stats, event ordering, graceful-close and bounded endpoint-drop follow `Endpoint`.

One connection deadline includes DNS, QUIC/TLS, HTTP/3 CONNECT, stream opening and
hello. Diagnostic errors omit URL/path/query and reflected peer errors. The
client uses pinned `wtransport 0.7.2` with only `ring` and QUIC-access support;
no new transport feature gate is needed in the existing native-only net crate.

Focused verification: `cargo test -p orr_net --test wt_client`, existing QUIC/WS/WSS
transport tests, and `cargo test -p orr_server --test net_e2e two_clients_over_native_wt`.
The latter runs two actual clients through Playing and checks agreeing checksums,
zero decode errors and zero desyncs.
