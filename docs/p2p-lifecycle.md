# Scoped P2P lifecycle integration

`orr_relay_net::NetEndpoint::try_send_reliable(conn, bytes)` returns the installed
`Transport`'s immediate enqueue result, preserving errors such as `Backpressure`
and `TooLarge`. Unknown or retired adapter connections return
`UnknownConnection`. Success means queue acceptance, not peer delivery. The
legacy `orr_proto::Endpoint::send` behavior is unchanged. A membership caller
can retain controls safely by using the fallible API:

```rust,ignore
let progress = membership.flush(|conn, channel, bytes| {
    assert_eq!(channel, orr_proto::Channel::Reliable);
    endpoint.try_send_reliable(conn, bytes)
});
```

Wrappers keep their own semantics. In particular, the optional network
conditioner accepts into its simulated queue and may hide later underlying
transport send errors. This API does not turn conditioner acceptance into native
socket acceptance or a delivery acknowledgment.

## Deterministic integration scenario

The private `server_ep::p2p_lifecycle_tests` module uses real `NetEndpoint`,
`P2pMembership`, `JoinBootstrap`, `Session`, `LocallyVerifiedHistory`, and
`DepartureBarrier` implementations. Paired mock `orr_net::Transport`s carry
actual control and input bytes. Application code in the fixture explicitly
admits the complete roster, scopes input links, and coordinates every participant.

The scenario covers:

- Two active peers and one committed vacancy; checked attempt A receives a
  snapshot and only one peer notice. Real endpoint backpressure retains the
  other peer's queued controls. A companion test retries distinct snapshot and
  notice payloads in order while an unblocked destination continues to progress
- Cancellation before Ready; bootstrap notice/session buffers are cleared,
  exact router-owned retention is released, and every explicitly owned join
  link is fenced at the endpoint. Buffered and late traffic is discarded;
  the existing shared link remains usable
- Explicit vacancy recovery by the sole default-input owner, followed by
  ordinary history pruning. Hold cleanup **alone does not authorize reuse**:
  this fixture establishes that the joiner emitted no input, none was admitted
  at either existing peer, all old participant links are fenced, and the
  recovery boundary is after verification and no later than the next send tick
- Fresh links and a fresh checked generation B. Late A controls and repeated A
  cleanup cannot alter B. Ready is distinguished from verified catch-up by
  deliberately delaying accepted input bytes until the existing peers have
  verified the target, then delivering the bytes and checking catch-up;
  endpoint ownership is relinquished before membership promotion
- A later departure with unequal received tails. The original author's links
  are fenced before trusted repair from locally verified history. A barrier
  requires the complete survivor roster and matching verified checkpoints;
  only its designated owner begins default inputs at the agreed cutoff

Complete per-slot input histories, exact ordered command bytes (including
repeated commands), and every available common checkpoint are compared with
an independently stepped headless Arena simulation.

## Boundary of this proof

The `TEST` Arena input codec, input-link routing, admission decisions, vacancy
recovery coordination, and trusted repair orchestration exist only in this
private test module. This is not production discovery, authentication, an input
or repair protocol, distributed consensus, or a runnable end-user P2P driver.
The paired transports do not demonstrate real-socket timing or readiness.
This scoped integration does not close the broader P2P feature.

Run the focused scenario with:

```sh
cargo test -p orr_relay_net --release --lib server_ep::p2p_lifecycle_tests
```
