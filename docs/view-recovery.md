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

This recovery implementation covers `InProc` and `Threaded`, including consumers
of their view streams. `RemoteBridge`, including the editor's in-process
`LocalHost` transport, retains its existing unbounded event queue and best-effort
split snapshot/event delivery. ERP sends events before throttled frames without
a shared event cursor, so no coherent bounded recovery is claimed there.
Network disconnection still requires the existing reconnect path. A future ERP
extension needs a snapshot/event fence and timeline identity before applying this
contract across that transport.

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
