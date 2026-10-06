# Fixed three-peer direct-QUIC input mesh

This is an opt-in fixed-roster foundation alongside the existing two-peer
`p2p_input` driver. The original finite constructor and smoke run remain the
default; an explicit rolling mode retires old driver bookkeeping under fixed
record and encoded-byte caps. It runs exactly three Arena participants over three direct
QUIC connections: **0–1, 0–2, and 1–2**. It does not forward participant 1's
inputs through participant 0. The older two-peer example and wire remain
unchanged.

```sh
cargo run -p orr_relay_net --release --example p2p_mesh_arena -- smoke
cargo run -p orr_relay_net --release --example p2p_mesh_arena -- rolling
# Optional: 4,096 live ticks, following a 768-tick two-peer prelude.
cargo run -p orr_relay_net --release --example p2p_mesh_arena -- rolling 4096 768
cargo test -p orr_relay_net --release --example p2p_mesh_arena
cargo test -p orr_relay_net --release --test p2p_mesh_input
cargo test -p orr_relay_net --release --test p2p_mesh_rolling
```

The runner is a single-process, headless local demonstration. Each participant
has its own `Session`, source, driver, membership registry, and verified
checksums. All transport data crosses actual loopback QUIC sockets, using the
listener's generated SHA-256 certificate pin. A trusted local orchestrator
supplies the topology, peers, and immutable generations. This is not production
client authentication: a certificate pin authenticates the server, and a
public generation number is a fence, not a credential. There is no address
lookup, NAT traversal, rendezvous, relay, or network-wide admission protocol.

## What a successful run proves

1. Participants 0 and 1 begin active with a three-slot Arena configuration.
   Only participant 0 authors default inputs for vacant slot 2. They exchange
   inputs over 0–1 and verify a 24-tick finite prelude, or a 640-tick rolling
   prelude by default. The rolling prelude retires old evidence before peer 2
   or its two direct connections are admitted.
2. Two new pinned connections complete the topology. Every participant freezes
   the same checked-v3 manifest: three slots, active peers `[0,1]`, donor 0,
   joiner 2, one generation and attempt 1. Transport/membership admission is
   separate from enabling mesh input on an edge.
3. Participant 2 sends checked ORRQ to donor 0. Donor 0 accepts it, installs the
   exact checked ticket in its input driver, and sends checked ORRJ to **both**
   1 and 2. Participant 1 imports that ticket from its admitted donor route.
   Participants 0 and 1 each install their own exact retention hold and send
   their own ORRB notice and authored backlog to 2.
4. Participant 2 initially services only its donor edge. The other edge remains
   in its bounded native transport event queue, rather than an application
   staging vector. Immediately after accepting the donor snapshot, it calls
   `install_joiner_snapshot` on that exact bootstrap/source before any advance
   or source polling, then enables both input edges. It requires both peer
   notices before Ready. New edge admission backfills only records authored
   locally, newer than the accepted snapshot. It never re-authors or forwards
   received records.
5. The joiner starts its scripted local input at the checked ticket's exact
   `first_input_tick`. The default owner stops authoring slot 2 at that
   boundary. With the current prelude and input delay 2, the snapshot is tick
   24 and first joiner input is tick 27, so the retained backlog is nonempty.
6. All three participants verify at least 120 finite-mode live ticks, or at
   least 2,048 rolling-mode live ticks, from that handoff.
   Every available verified checksum is compared to a separate three-player
   Arena simulation scripted from tick zero; the reference never copies the
   donor snapshot. Every active input tick carries three ordered commands,
   including a repeated owner: `[slot, (slot+1)%3, slot]`.
7. Each participant sends its own final report directly to its other two
   participants. Each independently requires reports from **all three fixed
   slots** at the same verified tick and checksum. Both directions of 1–2
   must carry at least the requested number of live input records.
8. Only then does application orchestration explicitly promote joiner 2 in
   all three membership registries. It applies each existing participant's
   exact captured hold cleanup and verifies no hold remains. A hold may
   already have ended normally when joiner input arrived. All three edges are
   room-shared; none was claimed as an exclusively join-owned transport, so
   promotion preserves every edge.

The default run prints `MESH_JOIN_READY notices=2 caught_up=false`, three
`MESH_VERIFIED` records, and `MESH_SMOKE_OK` only after these checks. Ready
advertises complete backlog coverage; it is not proof that those inputs have
arrived or that a target tick is verified. The delayed-backlog socket test
holds participant 1's input queue to 2 while 1–0 continues: 2 becomes Ready,
0 verifies live ticks, and 2 cannot verify those ticks until 1–2 is released.

`smoke [live_ticks]` defaults to 120 and accepts 120 through 484. This is a
finite demonstration, not rolling retention. All authored input must remain
in ticks 1 through 512, and authoring is limited to eight ticks ahead of local
verification, independently of the predicted-head limit. The simulation stops
advancing at the common target; input delay may produce a two-tick tail.
Each connection and checked-join phase has its own 30-second deadline. A
30-second absence of local verified progress fails the prelude or live phase;
there is no total-run duration deadline. Once all three local target reports
exist, the direct report exchange has a separate five-second deadline.
Any failed edge, hard send error, capacity exhaustion, malformed/unauthorized
input, missing peer, missing report, or timeout fails the run. There is no
roster shrink, survivor election, disconnect default takeover, or reconnect.


## Explicit rolling run and scope of the memory bound

`rolling [live_ticks] [prelude_ticks]` defaults to 2,048 live ticks after a
640-tick prelude. It requires at least 2,048 live ticks and a prelude greater
than 512. Target, input-delay tail, future horizon, and author-ahead arithmetic
are checked before starting. The author remains at most eight ticks ahead of
its local verification, even during delayed catch-up. For the default rolling
run, the snapshot is tick 640, first joiner input is tick 643, and the common
verified target is tick 2690.

The runner uses a deliberately small four-tick recent window and a 32-tick
future allowance. Its fixed per-driver caps are:

- 96 canonical records and `96 * 256` canonical encoded bytes
- 96 incoming records and `96 * 256` incoming encoded bytes
- 128 destination obligations, including pending and queue-accepted records
- At most 128 pending packets globally and `128 * 256` pending encoded bytes
- 64 pending packets and `64 * 256` pending encoded bytes per direct edge

The runner samples and asserts these aggregate and per-edge caps during
admission, receive, flush, polling, and authoring. It prints `MESH_DRIVER_CAPS`
and one `MESH_DRIVER_STATS` line per peer with observed high-water marks,
retirement floors, and floor-advance counts. Successful completion requires at
least 100 observed floor advances per peer and final floors beyond the
prelude; reaching a long target without retirement cannot pass.

Rolling mode always delays participant 1's authored backlog on 1–2 while 1–0
continues normally. It releases that delay only after participant 1's
retirement floor covers its first post-snapshot backlog tick. The exact 1–2
obligation must still be Pending, while the old queue-accepted 1–0 metadata
for that same logical record must already be gone. Participant 2 is Ready but
cannot verify the missing ticks until the actual direct 1–2 stream is
released. `MESH_RETIREMENT_DELAY_PROVED` records this check. The run still
requires three independent final reports, live traffic in both directions of
1–2, ordered repeated commands, and the tick-zero independent reference.
Only then may it print `MESH_ROLLING_OK`.

Rolling mode also bounds retained **checksum diagnostics**. It uses one
independent `Simulation<Arena>` from tick zero across both phases, with at
most 64 pending reference checkpoints. It never seeds that simulation from
the donor or joiner frame. Through the snapshot, every checkpoint requires
actual comparisons from participants 0 and 1. The accepted participant-2
snapshot is separately compared against the independently computed snapshot
checksum before bootstrap polling or prediction. The checked grant sets the
joiner's first-input tick before the reference advances past that snapshot.
Every later checkpoint requires all three fixed participants, including
post-snapshot backlog ticks before the joiner's first authored input.

Each participant has an exact next-checkpoint cursor and comparison count.
The runner requires interval-one checkpoints: a missing peer checkpoint,
missing reference, mismatch, or gap is an error, never a skipped comparison.
It marks a participant only after comparing its actual checksum. It evicts
only a contiguous prefix compared by every required participant, and retires
all retained Session checksum logs through that same common floor. A delayed
or absent participant cannot be removed from this requirement. The delayed
1–2 probe also checks that participant 2 pins the diagnostic floor while the
mesh driver's independent floor advances past its still-pending backlog.

Prediction is admitted only through `min(target, common_floor + 64)`, in
addition to the existing author-ahead limit. Receive, polling and flushing
continue while authoring is gated. This bounds a later verification batch,
not just the reference queue: each participating Session retains at most 64
checksum entries. Reference generation and new-checkpoint comparison per
loop are bounded in checkpoint count independently of run duration. Both
reference and Session high-water marks are asserted throughout the prelude
and live phase. `MESH_DIAGNOSTICS` reports those marks, the common retirement
floor, and exact coverage: ticks `1..=target` for participants 0/1 and
`snapshot+1..=target` for participant 2. Final reports are formed only after
the target entries have been retired, using separately cached actual peer
checksums. The independently computed final checksum remains a separate
scalar used to validate all three reports at every recipient.

`Session::retire_checksums_through(through)` is opt-in diagnostic retirement
for direct P2P sessions. It inclusively removes retained entries through an
already-verified tick and returns the number removed. Relay sessions and
requests beyond local verification are rejected without mutation. Repeated
calls are harmless; default retention remains unchanged without calls.
`JoinBootstrap` forwards only this operation, returning `None` before it owns
a Session or after cancellation; it does not expose unchecked mutable Session
access. Callers must finish required comparisons or exports before retiring.
Retirement is not a permanent floor: restore/re-verification keeps its usual
behavior and may record those ticks again. Consumers must reset their own
comparison state after restore or replacement; this fixed-mesh runner does
neither. Relay consumers retain their original checksum-history semantics.

These are bounded rolling mesh-driver ledgers and bounded retained diagnostic
records, not a duration-independent whole-process, arbitrary Game, or generic
Session memory bound. Retired `Vec` allocations may retain their high-water
capacity; allocator/RSS reclamation is not promised. Simulation/game state,
decoded objects, allocator overhead, and native transport queues remain
outside these bounds. The separate transport and checked-control limits
below still apply. This is not a long-lived production memory certification.
Finite smoke mode preserves its existing complete checksum logs and finite
independent reference map.

## Transport identity and the public input API

The public module is `orr_relay_net::p2p_mesh_input`:

- `P2pMeshInputDriver<G,C>` and `P2pMeshInputSource<G,C>` share one lifetime
  with one Session; do not replace or restore the Session/source generation
- `new(local, context, limits)` preserves the finite 1-through-512 behavior
- `new_rolling(local, context, limits, P2pMeshRollingWindow { recent_ticks,
  future_ticks })` explicitly enables rolling retention; finite `first_tick`
  and `end_tick` do not set its tick horizon
- `P2pMeshInputLimits` gives separate canonical, incoming, destination and
  per-edge bounds
- `P2pMeshEdge` registers `connection`, `adapter_id`, `raw_connection`,
  `remote`, and immutable nonzero edge `generation`
- `P2pInputCodec<G>` is the existing explicit application codec trait
- `install_ticket(&CheckedJoinTicket)` is for existing participants 0/1
- `install_joiner_snapshot(&JoinBootstrap<...>)` validates the accepted
  snapshot and exact source identity for participant 2
- `admit_edge`, `resolve_connection`, `receive`, `flush`, `check`,
  `disconnected`, and accounting methods expose explicit application control
- `rolling_progress()` returns `Some((locally_verified_tick, retired_through))`
  in rolling mode and `None` in finite mode
- `retained_evidence()`, `incoming()`, `pending()`, and `edge_pending(conn)`
  return independent record/encoded-byte usage; `destination_records()` counts
  retained destination obligations

A `NetLink`'s raw connection ID is always 1, scoped to that adapter. The
runner therefore has an explicit injective `(adapter_id, raw ConnId)` registry
mapping six local adapter connections to distinct `ConnId`s. Incoming events
are resolved with the actual adapter instance and raw event ID; packet fields
never establish peer identity. Membership and input routing use these same
unique IDs. An edge's generation is independently agreed at its two ends,
not learned from an incoming packet. Unique numeric IDs alone do not prove
admission or authentication.

The pre-existing 0–1 input edge may be admitted before a join ticket. Any edge
to or from participant 2 requires the checked cutoff first. Calling
`admit_edge` for a new destination atomically schedules eligible local backlog;
calling `send_local` with that backlog again is unnecessary and does not
create another destination obligation.

### Rolling verification, retirement, and late admission

Only the exact source's `on_locally_verified` callback advances existing-peer
rolling progress. Prediction, input arrival, duplicate traffic, and successful
sends do not. Importing a checked donor ticket on participant 1 establishes
its immutable authority cutoff; it is not proof of that participant's local
verification and must not slide its future horizon. Only participant 2's
`install_joiner_snapshot` can initialize progress from the exact accepted
bootstrap/source snapshot. Both rules are asserted by the example.

The irreversible retirement floor advances from local verification and the
recent window. Old canonical duplicate evidence and queue-accepted destination
metadata may retire. Pending per-destination packets retain their immutable
encoded bytes until that exact destination accepts the send, even when their
ticks are already retired. Verification does not remove undelivered incoming
records. If verification occurs during a flush callback, the in-flight FIFO
head and its Pending obligation remain valid; successful late acceptance must
not resurrect permanent old delivery metadata.

A fresh checked snapshot must cover every locally authored tick whose
canonical evidence has already been discarded. A stale first ticket fails
with `SnapshotTooOld`; the application does not silently shorten backlog or
reconstruct it from Session history. After a valid ticket is installed,
post-snapshot locally authored evidence is pinned inside the same driver
budgets until admitting peer 2 atomically creates that edge's obligations.
The runner has no second backlog archive, copy, or re-authoring loop. The
original 0–1 edge must be admitted before its uncovered local history is
forgotten. A persisted vacancy high-water mark prevents a retroactive cutoff
from becoming valid merely because old slot-2 evidence was retired.

An authorized retired packet returns `P2pMeshInputAccepted::IgnoredRetired`,
not `Duplicate`. Context, transport identity, recipient, edge generation,
authority, framing, lengths, command counts, and vacant-slot restrictions are
still checked before that outcome; retired packets are not decoded by the
application codec or reinserted. Recent conflicts still fail. New future input
must remain within `locally_verified_tick + future_ticks`; arithmetic
exhaustion fails terminally rather than wrapping. The existing ORRM v1 wire
format is unchanged. Code matching the accepted enum exhaustively must handle
the new result variant.

Call `check` before and after every Session/bootstrap mutation. The legacy
`InputSource` trait has void methods, so it cannot directly return transport
or capacity errors. A terminal driver error makes the source inert; the
application must stop advancing. Fixed membership failure is terminal even
when the failed participant's last input was already queued elsewhere.

## Wire formats and limits

Mesh input uses a distinct **ORRM v1** envelope, separate from old two-peer
ORRI and checked-v3 ORRQ/ORRJ/ORRB. Fields are little-endian. The envelope
contains schema, checked generation/attempt, edge generation and recipient;
the canonical logical record contains tick, slot, input length, command count,
input bytes, and length-prefixed command bytes in original order:

```text
ORRM[4] | version:u16=1 | schema:u64 | generation:u64 | attempt:u32 |
edge_generation:u64 | recipient:u8 | tick:u64 | slot:u8 |
input_len:u32 | command_count:u16 | input[input_len] |
(command_len:u32 | command[command_len]) * command_count
```

The fixed envelope is 35 bytes and logical header is 15 bytes. Framing,
identity, recipient, authority, tick window, counts, lengths and budgets are
validated before application decoding. Complete consumption is required.

The Arena mesh codec's schema is `0x4152454e41330001`. Inputs are exactly
20 bytes: two signed i64 raw Q48.16 axes, then a u32 buttons word. Axes must
be in `[-65536,65536]`, and only bit 0 is permitted. `_pad` is omitted and
reconstructed as zero. Commands are exactly one u32 owner in `0..=2`.
Maximum input packet size is 256 bytes, with up to three commands of four
bytes each. This is an explicit portable input contract, not native-Pod
serialization. Existing snapshot compatibility requirements are unchanged.

The default finite driver bounds are:

- Tick interval `[1,513)`
- 1,536 canonical logical records, at most `1536 * 256` encoded bytes
- 1,536 incoming records, at most `1536 * 256` encoded bytes
- 3,072 lifetime destination records, counting both pending and queue-accepted
  obligations
- 3,072 pending packets globally, at most `3072 * 256` encoded bytes
- 1,536 pending packets and `1536 * 256` encoded bytes per edge

Logical duplicate evidence and destination delivery state are distinct. Queue
acceptance for one destination does not discharge another destination's
obligation, acknowledge network delivery, or permit evidence pruning. A
backpressured FIFO head remains unchanged, while the other edge still receives
its own flush opportunity. In finite mode the driver retains exact lifetime duplicate evidence and
local verification does not retire destination records. In rolling mode,
old evidence and queue-accepted metadata retire under the rules above; pending
output never retires merely because of local verification.
Budgets describe encoded retention, not total allocator or decoded-object
memory. Codecs are trusted application code.

Checked controls have independent bounds in this runner: 64 KiB per snapshot,
4 KiB total bootstrap notice storage, and `2 * 64 KiB + 2 * 4 KiB` membership
outbound bytes per participant. Native sockets use a 64 KiB maximum message,
128 event slots and a 256 KiB send queue each. These are separate from the
256-byte input packet limit. A compressed snapshot wire limit is not a decoded
frame allocation limit: the existing bounded LZ4 decoder allows up to **255
times the compressed block length**, subject to valid size-prefix and frame
validation. The 64 KiB wire cap therefore permits a worst-case decompression
allocation near 16 MiB, plus frame decoding and ordinary transport overhead.

The example-only **ORRF v1** completion report is exactly 42 bytes:

```text
ORRF[4] | version:u8=1 | author:u8 | generation:u64 | attempt:u32 |
build_id:u64 | verified_tick:u64 | checksum:u64
```

Reports are accepted only from their admitted direct author route, on the
reliable channel, for the frozen generation, attempt and build. Exact duplicate
reports are allowed; conflicts fail. This report is a local demonstration's
completion check, not a general consensus, repair, or authentication protocol.

## Focused verification

The example tests exercise actual pinned loopback QUIC for:

- The finite three-edge late join and independent Arena reference
- Rolling late join after tick 512, followed by at least 2,048 live ticks and
  repeated retirements under asserted small driver caps
- Delayed direct 1–2 backlog, proving Ready differs from verified catch-up,
  including a rolling delay held across retirement
- Loss of one live 1–2 edge in both modes, which must fail the entire fixed mesh
- A missing participant 2 report in both modes, which must not reduce the
  completion roster

Additional example tests check exact report framing/context/identity and
portable Arena codec values, command order and repetition. Public-driver tests
cover authority, cutoffs, duplicate/conflict handling, bounded accounting,
per-destination FIFO/backpressure, malformed input, adapter isolation and
terminal errors. Rolling-driver tests separately cover checked verification,
retirement, admission pins, stale snapshots, delayed obligations, reentrant
flushes, retired framing/authority, and exhausted budgets/horizons. These checks
are scoped to a fixed three-peer room; they do not claim arbitrary-N membership, discovery, distributed consensus,
repair, production security, or completion of the broader P2P feature.
