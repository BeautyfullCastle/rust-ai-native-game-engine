# Fixed three-peer direct-QUIC input mesh

This is an opt-in, finite foundation alongside the existing two-peer
`p2p_input` driver. It runs exactly three Arena participants over three direct
QUIC connections: **0–1, 0–2, and 1–2**. It does not forward participant 1's
inputs through participant 0. The older two-peer example and wire remain
unchanged.

```sh
cargo run -p orr_relay_net --release --example p2p_mesh_arena -- smoke
cargo test -p orr_relay_net --release --example p2p_mesh_arena
cargo test -p orr_relay_net --release --test p2p_mesh_input
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
   inputs over 0–1 and verify a 24-tick prelude.
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
6. All three participants verify at least 120 live ticks from that handoff.
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
Any failed edge, hard send error, capacity exhaustion, malformed/unauthorized
input, missing peer, missing report, or timeout fails the run. There is no
roster shrink, survivor election, disconnect default takeover, or reconnect.

## Transport identity and the public input API

The public module is `orr_relay_net::p2p_mesh_input`:

- `P2pMeshInputDriver<G,C>` and `P2pMeshInputSource<G,C>` share one finite
  lifetime with one Session
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
its own flush opportunity. The driver retains exact finite duplicate evidence;
local verification does not retire pending output or destination records.
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

- The three-edge late join and independent Arena reference
- Delayed direct 1–2 backlog, proving Ready differs from verified catch-up
- Loss of one live 1–2 edge, which must fail the entire fixed mesh
- A missing participant 2 report, which must not reduce the completion roster

Additional example tests check exact report framing/context/identity and
portable Arena codec values, command order and repetition. Public-driver tests
cover authority, cutoffs, duplicate/conflict handling, bounded accounting,
per-destination FIFO/backpressure, malformed input, adapter isolation and
terminal errors. These checks are scoped to a fixed three-peer finite room;
they do not claim arbitrary-N membership, discovery, distributed consensus,
repair, production security, or completion of the broader P2P feature.
