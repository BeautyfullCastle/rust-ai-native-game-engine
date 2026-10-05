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

## Runnable two-peer direct QUIC with bounded rolling input history

The headless `orr_relay_net` example now exercises a usable direct-socket path:

```sh
cargo run -p orr_relay_net --release --example p2p_arena -- smoke
# Optional longer run, crossing the former 4096-tick lifetime:
cargo run -p orr_relay_net --release --example p2p_arena -- smoke 5000
cargo run -p orr_relay_net --release --example p2p_arena -- host 127.0.0.1:7000
# Explicitly allow at most two replacement development clients before activation:
cargo run -p orr_relay_net --release --example p2p_arena -- host 127.0.0.1:7000 77 120 2
# In another terminal, use the address, SHA-256 fingerprint and generation the host prints:
cargo run -p orr_relay_net --release --example p2p_arena -- join <address> <fingerprint> <generation>
```

`host [bind] [generation] [live_ticks] [pre_activation_retries]` starts Arena
with active slot 0 and committed vacant
slot 1, advances 24 ticks before advertising, and keeps advancing while waiting.
The host generates a fresh nonzero generation by default. `live_ticks` defaults
to 120, must be positive, and is printed in the matching join command.
`pre_activation_retries` defaults to zero: recovery admission is opt-in, with
an explicit maximum number of replacement first-connected development clients.
Each initial/retry generation has a 30-second pre-activation wait limit,
independent of continuing verified progress; resource or tick limits can fail sooner.
`join <address> <fingerprint> <generation> [live_ticks]` must use the same run
length; `smoke [live_ticks]` supplies it to both peers. For a remote machine,
supply its reachable host address to the joiner; no address discovery, NAT
traversal or relay is attempted. Do not copy a wildcard bind address as a remote
destination. The direct client retains the existing QUIC SHA-256 certificate-pin
verification, including normal TLS handshake failure on a wrong fingerprint.

This development example admits its first actual connected client into slot 1.
The address, fingerprint and expected generation are application-supplied; the
host's admission policy is intentionally simple. A generation is not a secret,
and a server certificate pin is not client authentication. Do not expose this
admission policy as production authentication or an open public game service.

The two sides independently freeze the two-slot roster and generation/attempt
(the example uses attempt 1). Public checked-v3 ORRQ/ORRJ/ORRB messages handle
request, snapshot and notice. The accepted transport connection is registered
with `P2pMembership`, not inferred from the slot claimed by an input packet.
Host controls use `NetEndpoint::try_send_reliable`; client controls use the new
`NetLink::try_send_reliable`. Both expose immediate queue errors, leaving legacy
`Endpoint::send` and `Link::send` unchanged. A successful enqueue is not a delivery
acknowledgment; optional conditioner wrappers retain their documented weaker
acceptance semantics. The example deliberately uses direct, unconditioned QUIC.

After the checked grant, the driver drops pre-snapshot outbound records and
sends the actual remaining authored backlog, including host-authored vacant
slot inputs. Live input then follows on that same reliable ordered connection.
Every scripted active-player tick carries ordered commands with repeated owners
(`[0,1,0]` or `[1,0,1]`), including the nonempty snapshot backlog. Input delay 2
ensures the host has an actual retained tail beyond its verified snapshot.

The joiner prints `JOIN_READY caught_up=false` when coverage is advertised; it
continues receiving inputs and advancing before printing `JOIN_CAUGHT_UP` at the
verified target (`live_ticks` beyond the checked first-input tick). Each side
checks every locally available verified checkpoint against an independently
stepped Arena baseline. The final checksum is also exchanged with an
example-only, generation-fenced report/ack; the host waits for peer closure
after acknowledgment. `SMOKE_OK` therefore requires real loopback QUIC and
matching verified simulation results, not merely successful control exchange.
The example's `ORRA` v1 report format is documented beside its encoder; it is
neither an input packet nor a general repair/consensus protocol. The default
short smoke uses only eight recent evidence ticks and asserts both peers retire
multiple windows; its `SMOKE_OK` reports the floors and retained record counts.

### Public input API and wire v1

`orr_relay_net::p2p_input` exports `P2pInputDriver`, `P2pInputSource`,
`P2pInputCodec`, `P2pInputLimits`, `encode_p2p_input` and explicit result/error
types. Construct one driver/source pair for one Session lifetime. The host can
author into the bounded pre-admission queue. Bind once to the actual admitted
connection, the independently agreed `CheckedJoinContext`, the accepted
snapshot tick and first-input tick. The host derives these from its checked
ticket; the joiner derives them from its accepted bootstrap Session. Do not use
unvalidated incoming header fields as binding authority. This API accepts scalar
ticks, so the application must establish that validated-snapshot precondition;
context equality and structural checks alone do not authenticate a snapshot.

This format is separate from both relay traffic and checked-v3 controls:

```text
ORRI[4] | version:u16=1 | schema:u64 | generation:u64 | attempt:u32 |
slot:u8 | tick:u64 | input_len:u32 | command_count:u16 | input[input_len] |
(command_len:u32 | command[command_len]) * command_count
```

Every integer is little-endian. The fixed header is 41 bytes. Exact complete
consumption is required; trailing bytes, truncation, unsupported version/schema,
and malformed counts/lengths fail. The packet uses transport integrity rather
than claiming that a noncryptographic packet checksum authenticates a sender.
Commands retain their original order and repetitions.

`P2pInputCodec<G>` supplies a stable schema ID and explicit input/command
encoders and decoders. The application must specify widths, endianness, valid
values and canonical encoding, consume each complete payload slice, and change
the schema ID when that contract changes. The framing validates all counts,
lengths, tick/context/connection authority and retained capacity before invoking
application decoders or allocating decoded commands. Codecs are trusted code;
the driver cannot bound allocations a malicious codec chooses to make.

The example Arena schema `0x4152454e41000001` encodes raw Q48.16 axes as two i64
LE values followed by a u32 LE buttons word: 20 bytes. Axes are within
`[-65536,65536]` raw, only button bit 0 is valid. Rust `_pad` is omitted and
reconstructed as zero. A command is exactly one u32 LE owner in `0..=1`.
No generic raw-`Pod` portability promise is made. Existing snapshot and
`SimCommand` encodings retain their own compatibility requirements; this input
codec does not redefine them.

### Authority, backpressure, bounded failure and cleanup

Only two peers are supported: host 0 and joiner 1. After binding, records must be
strictly after the snapshot to enter Session. Host 0 may supply its own slot at any allowed tick,
and slot 1 only before the ticket's `first_input_tick`. Joiner 1 may author slot
1 starting exactly at that tick. Neither side may claim any other slot. The
pre-admission host backlog is revalidated against that cutoff during binding.
Each receive validates the actual admitted connection, reliable channel,
generation, attempt and application schema before Session can see the record.
Equal numeric connection IDs from a different adapter are not authentication;
applications must preserve the demonstrated one-adapter namespace or remap IDs.

There are two explicit source modes, sharing unchanged ORRI v1 framing:

- `P2pInputDriver::new` preserves the finite window (`first_tick..end_tick`,
  default `1..4097`) and exact duplicate evidence for that entire lifetime
- `P2pInputDriver::new_rolling` takes
  `orr_relay_net::p2p_input::P2pRollingWindow { recent_ticks, future_ticks }`.
  Both distances must be positive. In this mode `P2pInputLimits.first_tick` and
  `end_tick` are ignored; its packet, field, record and byte budgets still apply.
  The standalone `encode_p2p_input` helper retains its finite range checks

Rolling progress starts at zero. Only Session's `on_locally_verified` callback
or the application's validated snapshot binding may advance it. The narrow
cancelled-host replacement helper below also seeds it from the continuing
Session's own verified checkpoint. Packet maxima,
predicted head ticks and peer progress reports do not move it. Allowed future
ticks are at most `progress + future_ticks`, inclusive. Both this addition and
two ticks of cursor headroom are checked; `TickExhausted` terminates rather
than wrapping or saturating the future horizon. The inclusive horizon cannot
exceed `u64::MAX - 2`: even with stalled verification, Session can pre-increment
its send cursor for the first rejected send before the required driver check.
`future_ticks` values of `u64::MAX` and `u64::MAX - 1` are therefore invalid at
construction.
Out-of-window future traffic fails with `TickRange`, even if individually valid.

The irreversible retirement floor is the maximum of the accepted snapshot tick,
its previous value, and `progress.saturating_sub(recent_ticks)`. Thus a window of
one retains the current verified tick's evidence. At or below the floor,
otherwise valid traffic yields `IgnoredRetired`, explicitly distinct from an
exact duplicate. Connection, channel, version/schema, generation/attempt,
origin/cutoff authority, complete framing, and packet/field byte bounds are all
checked first. Retired traffic never calls application decoders, enters Session,
or recreates duplicate evidence. It may contain different bytes than the old
record; after retirement the source makes no equality or conflict claim.

Above the floor, exact duplicate wire records are harmless, including after
polling. A differing record for a seen `(tick, slot)` fails explicitly, including
changed command order. The source exposes `retained_evidence()` as record/byte
counts and `rolling_progress()` as `(watermark, retired_through)` for diagnostics.
These are observations, not a way to externally advance the window.

Before admission, the rolling host may discard verified outbound records because
a fresh checked snapshot covers them. Binding with a snapshot older than any
such discarded record fails `SnapshotTooOld`. Required post-snapshot output is
retained in FIFO order and revalidated against the checked cutoff. After binding,
local verification never discards unsent output, even when its duplicate evidence
has retired: verification is not peer receipt. A blocked FIFO head survives
`Backpressure`; a retry processes only the queue length at flush start. Callback
inspection and verification are safe, cancellation halts the flush, and recursive
flushing is terminal. Capacity exhaustion fails visibly rather than losing output.

Each of the duplicate ledger, outbound queue and incoming queue has independent
`max_records` and `max_retained_bytes` limits (default 8192 records and 2 MiB).
Packet/input/command/count limits are independent. Encoded retained payload is
at most three times that byte budget, excluding temporary codec/transport copies;
decoded commands use at most `max_records * max_commands` objects. The example
uses 192 records / 48 KiB per structure, a 256-byte packet, 20-byte input, at most
three four-byte commands, eight recent ticks and a 64-tick future horizon.

These are source-memory bounds, not a whole-session or process bound. In
particular, the example's independent reference checksum vector and Session's
checksum log grow with run length. Long-running bounded whole-session history,
replay guarantees, established-player reconnect, acknowledgments of individual inputs and repair
remain separate work; the rolling source adds no new wire messages.

Exhaustion, recent conflicts, unauthorized traffic or a non-backpressure send
error make failure sticky, clear buffers and require terminating the session.
Check `driver.check()` before and after Session/bootstrap calls because the
legacy InputSource methods return no error. A driver/source belongs to one
Session lifetime: cancel and replace it before restoring/replacing the Session,
including a forward restore; a lower callback cannot lower its floor.

The example bounds transport message size, connections, event/send/control
queues and stalled progress. Its 30-second watchdog is reset by verified progress
or actual bootstrap/completion transitions, not arbitrary incoming traffic;
there is no total 60-second run deadline. Separately, each initial/retry generation
has a fixed 30-second pre-activation wait cap, even while the vacant host advances.
Rolling sources still support joins after the former fixed tick horizon; the
bounded example wait is an application policy. Snapshot controls retain existing
checked wire/decompression bounds, distinct from the input-packet limit.

Bootstrap disconnect/error uses the existing owned cleanup: cancel or invalidate
the bootstrap, release the donor's exact local hold, retire only its registered
exclusive transport lease, and cancel the driver's application buffers.
`NetLink` now supports the same identity-owned retirement as `NetEndpoint`;
foreign, stale or relinquished leases cannot close another link with the same
numeric ID. Retirement fences late buffered input immediately. It cannot retract
already enqueued remote bytes.

### Explicit pre-activation retry, preserving the host world

With a nonzero `pre_activation_retries` argument, the host can recover a lost
joining link only before its final checked ORRB notice was successfully enqueued
by `try_send_reliable`. This exact enqueue is latched in the flush callback.
Creating a grant, queueing a control in membership, or receiving no joiner input
is not evidence that the joiner is still unready: a successfully enqueued notice
may already let it become Ready and author inputs. After that boundary the demo
fails closed on disconnect, even when the host received no ORRI. Normal completed
report/ack shutdown remains successful.

Before recovery the host fences all events from the old ConnId, cancels the
membership attempt, releases its exact owned hold, retires its exclusive link,
and cancels its old input driver. `replace_cancelled_host_rolling` verifies that
the driver is the continuing Session's cancelled rolling host source, that no
remote input was ever admitted (including input not yet polled into Session),
and that the generation is fresh. It validates the retained pending grant and
vacancy cutoff, seeds progress from `Session::verified_tick()`, and reconstructs
all required unverified locally authored records using `authored_since`.
Missing history, incorrect authority, stale context/cutoff, tick exhaustion or
insufficient queue capacity fails before replacing the source or resuming defaults.
The capacity preflight includes the exact default records `mark_slot_vacant`
will immediately emit. The helper cannot prove the transport/readiness policy;
its caller must enforce the no-successful-final-notice boundary and old-link fencing.

Only after this transactional preflight is the fresh source installed in the
same live Session. The host then marks the former grant vacant at its original
first-input tick and resumes default inputs. Its world, checksums, authored
commands and send cursor are not reset. If no request had reached the host, the
same helper preserves both slots' already-authored tail without re-vacating it.
The endpoint and certificate fingerprint stay unchanged, the generation increases
with checked arithmetic, and a fresh `JOIN_COMMAND` is printed. Run that new
command to join through a fresh link; the failed client is not silently re-used.
No additional client identity is admitted beyond the explicit retry budget.

This deliberately excludes established-player reconnect. Accepted unverified
remote tails can carry commands that defaults cannot replace; they need a separate
agreement/repair policy. The example never silently drops them to reopen a slot.

Focused checks:

```sh
cargo test -p orr_relay_net --release --all-targets
cargo clippy -p orr_relay_net --release --all-targets -- -D warnings
```

Rolling regressions compare both peers against independent simulation for 4300
ticks, exercise checked late join after 4300 pre-admission ticks with the exact
post-snapshot FIFO tail, and test stalled verification, recent conflicts, retired
framing/authority checks, queue budgets, cancellation and cursor exhaustion.
New source-replacement regressions cover retained ordered/repeated commands,
missing history, exact record/byte preflight (including vacancy defaults), stale
handles, and both polled and unpolled remote input rejection. Real-QUIC tests
accept a first snapshot after host progress beyond the initial rolling horizon,
disconnect before its final notice, and join the same continuing world over a
second fresh link. Both peers match independently simulated checkpoints using
only the successful grant's activation boundary. A separate real-QUIC test closes
a Ready joiner before sending any input and proves that retry remains forbidden.
Old input/control/disconnect routing is fenced, and retry admission/wait is bounded.

The earlier real QUIC `smoke 5000` run also verified tick 5028 with matching independent
checkpoints and eight-tick-window retirement floors at 5020. These results do
not imply bounded Session checksum/reference history.

Remaining scope includes production admission/authentication, discovery/full
mesh, established-player reconnect or multi-peer vacancy recovery coordination, a repair wire protocol,
whole-session bounded history and datagram redundancy. No simulation
was added to the TUI. This real two-peer path advances issue #12 but does not
claim that the complete P2P feature is finished.
