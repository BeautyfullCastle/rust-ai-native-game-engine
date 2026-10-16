# Native relay WSS client

Native samples can connect to the existing secure WebSocket listener:

```sh
orr_server --transport quic --bind 127.0.0.1:4000 --wss-bind 127.0.0.1:4443 --tls-cert server.pem --tls-key server.key
arena --connect wss://relay.example:4443/relay --transport wss --headless --bot
# Private CA or local test certificate:
arena --connect wss://localhost:4443/relay --transport wss --ca-cert ca.pem --headless --bot
```

Check `orr_server --help` for deployment flags. Configure the existing QUIC
server TLS identity and `wss_bind`; standalone `listen(Wss)` is intentionally
unsupported and returns that configuration guidance. No new server, ERP token,
relay authentication mechanism, or FFI transport value is introduced.

`ConnectOptions::new(address, TransportKind::Wss, Trust::WebPki)` accepts a
`host:port` or complete `wss://` URL, preserving its path and query. WSS uses
public Mozilla roots by default in the sample CLI and checks the URL's DNS name
or IP address. `--ca-cert PATH` / `Trust::PemFile` explicitly select PEM CA trust;
certificate chain, name, signature and validity checks remain enabled.
`--server-name NAME` (or nonempty `ConnectOptions::server_name`) explicitly
overrides the certificate name, without changing the URL or HTTP Host header.
QUIC retains its previous `localhost` default and trust policies.

At the transport level use
`Endpoint::connect_wss(url, server_name: Option<&str>, trust, cfg)`.
`QuicTrust::WebPki`, `CertDer`, and `PemFile` are accepted. Fingerprint-only and
insecure trust are rejected for WSS; there is no certificate-verification bypass.
Plain `Endpoint::connect_ws` is unchanged. WSS negotiates HTTP/1.1 over TLS 1.2
or 1.3; QUIC retains TLS 1.3 and `orrery/1` ALPN.

DNS resolution, TCP connection, TLS handshake and HTTP upgrade run within one
`NetConfig::connect_timeout`. The separate sample `--connect-timeout` still
bounds waiting for the relay room to start. Connection diagnostics identify the
failed stage, excluding URL path/query, certificate paths and reflected server
responses. Userinfo and URL fragments are rejected. Do not put credentials in
URLs: they are still transmitted to the selected server as HTTP request data.

The native upgrade requires the server to select exactly `orrery/1`.
After TLS/upgrade the existing WS tagged binary framing, queue/backpressure,
stats, ping/idle and graceful close implementation is reused. Both channels
share TCP ordering; “unreliable” is not a native datagram on WSS.

Focused coverage is in `orr_net/tests/wss.rs` and
`orr_server/tests/net_e2e.rs::two_clients_over_native_wss`, plus option tests in
`orr_relay_net` and `orr_sample`. The E2E identity is a public test-only fixture,
not deployment key material.
