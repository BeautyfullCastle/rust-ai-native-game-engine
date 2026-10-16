# ERP client queue bounds

ERP clients retain requests, incoming messages and asynchronous RPC errors in
separate bounded queues. Admission never waits for channel capacity. A refused
outgoing request has not been enqueued; incoming loss or error overflow makes
the connection terminal. Accepted requests can already have executed when a
connection ends, so reconnecting does not resend them.

## Limits and byte costs

| Queue | Default items | Default retained cost | Cost of one item |
| --- | ---: | ---: | --- |
| PumpedWs outgoing | 4096 | 64 MiB | Compact JSON-RPC request text length |
| LocalTransport outgoing | 4096 | 64 MiB | Canonical compact JSON-RPC envelope length |
| PumpedWs incoming | 4096 | 64 MiB | Text or binary payload length |
| LocalTransport incoming | 4096 | 64 MiB | Text/wire length or existing `LocalFrame.cost` |
| RemoteBridge RPC errors | 256 | 1 MiB | Compact JSON of the `RpcError` object |

64 MiB matches the existing host waiting-request byte budget and leaves room
for normal payloads relative to the default 4 MiB server request message limit.
It is a finite retention policy, not a measured product memory target.
Zero in either dimension refuses every item, including a zero-byte item. Every
admitted item consumes one slot. Both dimensions are checked with overflow-safe
arithmetic inside one short mutex acquisition.

The local request is also subject to the existing global host request count
and byte limits. Its existing `RequestPermit` owns the per-client reservation
through the producer, host inbox and stash. These are separate admission scopes
over the same waiting request, rather than two physical copies. Dequeuing into
active host execution releases both. Local incoming `ConnTx::pending()` reads
the single incoming queue budget; it does not add another byte charge.

Payload cost is not allocator or process memory. It excludes serialization and
parse/decode temporaries, one active socket write, active RPC execution,
channel/JSON/allocator overhead, caller-held messages and snapshots, and the
public queues owned by `ErpClient`. A shared local frame can be charged for each
queued message even when the frame's `Arc` shares storage. Stage costs must not
be added into a process peak or described as private heap or RSS.

The network server's existing outgoing presentation/control queues retain their
own policy. This change does not establish a new server socket-egress item
limit, FFI limit, global connection-event limit, or a whole-process memory bound.

## Refusal, termination and ownership

`TxHandle::try_send` returns `SendError::Backpressure` before an outgoing
request enters the queue. `RemoteBridge::request` maps that to the existing
`BridgeError::Backpressure`. The older `send` API reports a transport error.
The transport stays available after an ordinary outgoing capacity refusal;
the caller can choose a later new attempt. There is no automatic retry,
reordering, deduplication or reconnection policy in the queue.

A retained envelope owns a reservation. Dequeue, failed channel delivery and
receiver/worker destruction release that reservation. Closing a queue stops
admission and wakes its worker, but does not reset live accounting while an
envelope or host permit still owns bytes. `current_*` returns to zero when that
ownership actually ends; `peak_*` and the saturating saturation counter remain.
The socket worker bounds an active write to 30 seconds and also observes an
explicit close during connection establishment or a write.

An incoming capacity refusal closes the connection. This applies to replies,
notifications and view frames alike. `recv` returns a terminal transport error
and releases the undeliverable retained tail. It does not return an incomplete
tail as a healthy fenced stream. RemoteBridge disconnects/fail-closes its view
mailbox through the existing fenced delivery path. A newly constructed bridge
negotiates a fresh fence and snapshot; the queue does not manufacture missed
events or replay accepted edits. Connection termination is the terminal outcome
for outstanding calls, not evidence that an accepted edit was rolled back.

Dropping `PumpedWs` or calling its sender's `close()` terminates its worker even
when a sender clone survives. Worker destruction releases waiting outgoing
envelopes. Local transport destruction marks both budgets terminal and reports
the existing disconnect to the host. Previously accepted local requests can
remain in the host inbox until the host dequeues or drops their owned permits.

RPC error overflow preserves already retained errors and a fixed-size sticky
terminal marker, closes the transport and stops the bridge. `take_errors()`
drains retained errors and generates one separate
`remote_error_queue_overflow` diagnostic for the caller. That diagnostic is not
charged as another retained queue entry and is reported once. Overflow cannot
silently evict refusals while leaving the connection healthy.

## Configuration and observation

The public types are in `orr_remote::link`:

```rust
use orr_remote::link::{QueueLimits, TransportQueueLimits};

let limits = TransportQueueLimits {
    outgoing: QueueLimits { max_items: 256, max_bytes: 8 << 20 },
    incoming: QueueLimits { max_items: 512, max_bytes: 16 << 20 },
};
```

`LocalConnector::connect_with_queue_limits` and
`PumpedWs::connect_with_queue_limits` accept transport limits. RemoteBridge
adds `connect_with_queue_limits(config, transport_limits, error_limits)` and
`connect_transport_with_error_limits(transport, config, error_limits)`.
Original constructors use the defaults. The socket parser's maximum frame and
message payload lengths also use the configured incoming byte limit.

`Transport::queue_stats()` and `TxHandle::queue_stats()` expose independent
outgoing/incoming `QueueStats`. RemoteBridge provides
`transport_queue_stats()` and `error_queue_stats()`. A stats snapshot contains
limits, current/peak items and bytes, saturations and a terminal `closed` flag.
Snapshots of two different queues are not an atomic global memory sample.

`RemoteConfig` and `RemoteMetrics` keep their struct-literal fields. Existing
custom `Transport`/`TxHandle` implementations compile through default trait
methods; their default stats are unavailable, and their old send errors map to
`Disconnected`. They acquire bounds only if their own transport implements
them. The blocking `WsTransport` has no retained worker queues and is unchanged.

## Focused verification

The new tests in `tests/client_queue_bounds.rs` exercise accepted local edit
order and refusal without duplication, slow local/socket consumers, retained
large payloads, terminal socket-worker shutdown with a surviving sender, local
fenced reconnection to the latest checksum, and local/socket RPC-error overflow
and reclamation. `src/link_tests.rs` also checks concurrent dual-dimension
admission, zero/exact/oversized/overflowing costs, failed channel delivery and
request-permit ownership. Existing server loss-injection fixtures retain their
test-only constructor contract; production LocalConnector uses bounded ingress.

```text
cargo test -p orr_remote --release --locked --offline --lib --test client_queue_bounds --test local --no-run
cargo test -p orr_remote --release --locked --offline --lib -- --test-threads=1
cargo test -p orr_remote --release --locked --offline --test client_queue_bounds -- --test-threads=1
cargo test -p orr_remote --release --locked --offline --test local -- --test-threads=1
cargo clippy -p orr_remote --release --locked --offline --all-targets -- -D warnings
```

These are correctness checks, not a latency, performance or memory benchmark.
Assertions, golden checksums and required CI remain in force.
