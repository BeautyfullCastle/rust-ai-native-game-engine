# Bounded presentation-event recovery

`InProc` and `Threaded` keep running when their view stops consuming events.
`BridgeConfig::view_event_capacity` (default 4096, minimum 1) bounds retained
**presentation notifications**, including asynchronous lifecycle diagnostics.
A notification batch is one complete simulation/control outcome. An oversized
batch or a batch that does not fit is discarded as a whole.

This limit does not bound simulation state, the current outcome produced by a
simulation step, snapshots retained by callers, input/command queues, RPC
responses, or network transport buffers. It is not an engine-wide byte budget.

## The recovery contract

Use `Bridge::poll_view()` rather than separate snapshot/event reads. It returns
`ViewUpdate { snapshot, events, resync }`.

- Normal delivery preserves event order. A normal snapshot can be newer than
  its notification tail, as before.
- Overflow advances a presentation-delivery generation. Recovery pins one
  complete publication containing a snapshot and an independent output cursor.
  The snapshot is taken after every discarded outcome; it is not paired with
  an arbitrary later read of another slot.
- The view discards the event prefix through that cursor. Even if a producer
  enqueues a reserved batch after the recovery read, its old cursor is skipped.
  Only a later output cursor can subsequently be delivered.
- Reading drains at most the configured number of notifications and samples
  the newest publication once. There is no retry-until-caught-up loop. The sim
  may advance throughout recovery; the next render simply reads a newer state.
  A view does not replay missing simulation ticks.
- Recovery clears the speculative-effect table, interpolation history, and
  pending presentation lifecycle. Persistent visuals are rebuilt from the
  snapshot. Missed transient sounds/VFX are intentionally not replayed.
  A persistent effect that must survive recovery must be represented in
  snapshot-backed state; an event-only lifetime cannot be reconstructed.
- Late predicted/verified/canceled notifications originating at or before the
  recovery head are suppressed, so an old effect cannot restart after its
  predicted handle was cleared. This tick floor is scoped to the ordered
  batch's timeline epoch and is cleared at a subsequent timeline discontinuity.
  The simulation's prediction/rollback state is not reset.

`ViewWorld` and `ViewWorld3` provide `update_from_bridge()` and
`reset_from_snapshot()`. Recovery immediately displays the baseline's current
predicted pose, rather than blending from an old pose or showing its previous
frame. Snapshot-mode entities retain their verified-state semantics. Application
sound/VFX owners must clear their own speculative handles too.

The legacy `drain_events()` API emits an explicit
`BridgeEvent::ViewResynced(ViewResync)` marker. Callers must handle that marker,
clear presentation state, and obtain a fresh snapshot. Prefer the pinned
`poll_view()` result; wrappers using its default implementation retain their
adapter's legacy split-read guarantees.

## Lifecycle diagnostics and durable consumers

A reset contains bounded diagnostic summaries, not a replay of the normal FIFO.
There are twelve fixed categories: session started, rollback, stalled, desync,
disconnected, delay changed, seeked, branched, paused, resumed, debug rejected,
and seek rejected. Each retains a saturating `u64` occurrence count and its most
recent fixed-size value. Summaries are ordered by their last occurrence; they
cannot reconstruct every intermediate transition or every rejected target.
Terminal disconnect and the latest desync tick are additionally sticky in the
reset, including when already observed before the overflow. A view reset does
not reconnect a dead session. Read current timeline state for playing/paused and
use the adapter's connection status rather than replaying coalesced transitions.

Normal synchronous error results and ERP request/response handling are unchanged.
The summaries describe asynchronous diagnostics only, not individual request
acknowledgements. Do not treat a coalesced `DebugRejected` or `SeekRejected` as the
complete response history.

`Verified` means the simulation will not roll that event back. It does **not**
mean the presentation notification is durable. Achievements, accounting,
persistence, or required audit records must consume a reliable authoritative
path upstream of this lossy view mailbox. For example, Arena updates its `Score`
singleton before emitting `Hit`, so missing a visual hit notification does not
lose the authoritative score in the snapshot.

## Adapter and wire limits

`InProc`, `Threaded`, and an explicitly negotiated `RemoteBridge` support
bounded coherent presentation recovery. `RemoteConfig::view_delivery` selects
`RequireFenced`, `PreferFenced` (default), or `Legacy`.
`RemoteBridge::view_delivery()` reports the guarantee the host actually
acknowledged. An old host may ignore the extra subscription parameter; a
successful subscription without the explicit acknowledgement is **legacy**,
never evidence of fenced recovery. `RequireFenced` rejects that connection.
`PreferFenced` preserves the old unbounded, split-read behavior on an old host;
the editor displays this fallback. Local editor hosts require the extension.

The negotiated mailbox's `view_event_capacity` bounds the combined retained
notification staging and render queues, with two fixed local status slots for
session start and terminal disconnect. Transport ingress (`PumpedWs` and
`LocalTransport`), RPC requests/replies, `take_errors`, and caller-retained
snapshots are outside this bound. Oversized decoded messages have a temporary
allocation cost. This is not a whole-transport memory or byte budget.

The ERP `watch.subscribe` request opts in with `view_delivery: 1` and all three
`frames`, `events`, and `notes` topics. The result explicitly echoes the version,
a new subscription identity, and initial cursor/count. Every negotiated
notification carries a `delivery` object with subscription, presentation
timeline, independent output cursor, and cumulative notification count. Full
snapshot metadata carries the matching `through_cursor`, loss generation, and
bounded cumulative lifecycle summaries. All delivery counters are decimal u64
strings; existing frame payloads and ERP/ORRS wire versions are unchanged.
Presentation identity includes session incarnation, seek/debug epoch, explicit
branch, and source changes; it is not the raw simulation tick or epoch alone.

Notifications wait in bounded staging until a decoded full frame covers their
output cursor. A frame's state and watermark are built together, including
paused same-tick diagnostic progress. A cursor gap, dropped transport admission,
timeline change, or local overflow requires a newest covering baseline. That
reset stays pending until `poll_view()` consumes its pinned snapshot, and newer
frames can replace it while simulation continues. Missing transient effects
are discarded; no replay/catch-up loop runs on the render thread. Late old event
keys are suppressed only within that presentation timeline.

When a requested source is unavailable (stopped `sim`, unavailable proposal),
`watch.view.inactive` carries the same complete delivery fence without a frame.
It explicitly clears stale presentation state. The next available source gets
a fresh timeline identity and baseline. Malformed negotiated metadata or payload
fails the presentation connection closed instead of silently downgrading it.
Disconnect is observable even while recovery awaits a frame; an older snapshot
never masquerades as a covering reset. Reconnect uses the existing explicit
path and starts a new subscription; it does not restart the simulation.

Six ERP play-note categories have cumulative fixed-size summaries (seek,
branch, pause, resume, debug rejection, seek rejection); local start/disconnect
status is separate. These diagnostics are coalesced history, never fabricated
individual RPC responses. Auth and request replies bypass presentation admission
loss, and remain outside its bounded queue claim. The editor polls both main
and proposal-preview streams, rebuilds from the pinned pair, and retains its
separate request-response bookkeeping.

Viewstream carries an explicit events-reset frame flag together with
`FLAG_DISCONTINUITY`; it forces a frame even if the paused snapshot sequence has
not changed. Updated readers must clear pending predicted-event handles and
interpolation history before ingesting the frame. Older readers that ignore the
new flag do not acquire event-recovery correctness automatically. Per-category
diagnostic counts are available in the Rust `ViewResync`, not the existing
binary frame layout. See [the view-stream format](view-stream.md) for flag values
and external-reader requirements.


### Downstream baseline acknowledgement

The FFI client frame slot retains a reset until `orr_view_poll` successfully
copies it or `orr_view_poll_ptr` takes it. Status/event polling and too-small
buffers cannot consume that reset. Newer frames replace the baseline with
`prev == cur`, queued events are cleared, and events remain suppressed until
the baseline is consumed. The delivered baseline's tick then filters late
old-tick records until a later normal discontinuity.

ERP client-mode fanout keeps the same reset pending per subscriber while its
frame is rate-limited or waiting behind transport backlog. The next deliverable
full snapshot inherits the reset; later event batches are filtered against the
actual delivered baseline. This relies on ordered delivery by the existing
reliable transport; it does not repair arbitrary network loss or replace a
reconnect protocol.

The FFI's independent existing cap of 4096 unread encoded event batches still
has its previous drop-oldest behavior. It is not the newly bounded Bridge
notification mailbox and does not gain a cursor-fenced reset on its own
capacity overflow. Applications must drain that event queue regularly; do not
claim that all FFI/backend queue overloads now recover coherently.

## Verification at this checkpoint

139 distinct focused tests passed in the native debug profile:

- `cargo test -p orr_bridge -p orr_view --lib --tests`: 54 tests
- `cargo test -p orr_viewstream`: 18 tests
- `cargo test -p orr_ffi`, the `orr_remote` library plus `local`,
  `view_recovery_remote`, `view_recovery_ws`, `viewstream`, and `watch_remote`
  integration targets, and `cargo test -p orr_tui`: 67 tests

The final `cargo clippy --workspace --all-targets -- -D warnings` also passed.
C-compiler-dependent tests ran with `ORR_REQUIRE_C_COMPILER=1`.

The new live WebSocket regression uses a capacity-two manually paced
`Threaded<PhysGame>` bridge, causes actual notification overflow, and verifies
that a one-fps subscriber receives the newest tick-104 reset baseline, with no
events ahead of that baseline and no later revival of the old event key. Only
the deliberately delayed old/new event-key pair is injected by the fixture;
the overflow, snapshot, source encoding, ERP host, and WebSocket are real.
FFI reset edge cases are tested directly at the mailbox helpers; existing C ABI
and multiplayer C/FFI integration tests cover the exported polling paths, but
the new reset itself has not been forced through an actual external C caller.

The complete `cargo test --workspace --release` aggregate was still compiling
and linking with thin LTO when this checkpoint was recorded. It is **pending**,
not a claimed full-workspace pass. The focused count above must not be described
as a full release, browser-runtime, or cross-platform verification result.


## Negotiated remote verification

85 distinct focused tests passed in the native debug profile for this extension:

- `cargo test -p orr_remote --lib --test remote_delivery --test remote_view_live`:
  44 tests (33 library, 9 negotiation/malformed-peer, 2 actual transport)
- `cargo test -p orr_editor --lib --test attach --test agent --test crash`:
  25 tests, including 8 main/preview recovery regressions
- `cargo test -p orr_remote --test local --test watch_remote --test view_recovery_remote`:
  16 compatibility tests

Both actual transport regressions use `LocalHost<PhysGame>` with a capacity-one
RemoteBridge and one-fps frames, once through its in-process transport and once
through a real WebSocket. Real shooting events overflow staging; the recovered
snapshot reaches tick 104 with the authoritative checksum. While presentation
is left unread the real-time simulation keeps advancing. Recovery then accepts
a genuine newer event tail without exposing an event ahead of its snapshot.
The malformed-peer fixtures additionally cover missing capability acknowledgements,
malformed fences, impossible verified progress, and disconnect before coverage.

The four `cargo test -p orr_mcp --test agents_md` checks also passed, including
identical regeneration of the committed guide (89 focused tests in total).
`cargo clippy --workspace --all-targets -- -D warnings` also passed for the
complete workspace. No authoritative simulation/checksum or late-join protocol
changes are included.

These focused results are not a full-workspace release, browser-runtime, or
cross-platform result. The historical checkpoint counts above predate the
negotiated extension. Full release/CI verification of this extension is separate.
