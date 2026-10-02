# Local command admission and replay read-only policy

`InProc` and `Threaded` share the same explicit-command admission budget.
`BridgeConfig::command_capacity` defaults to 1024 and `control_capacity` to 64;
both clamp to at least one, including when fields are assigned directly.

## Reliable admission

- An accepted explicit gameplay command consumes one permit until it has left
  the reliable mailbox, paused `SimCore` staging, and the host's unsubmitted
  staging. Dequeueing a message or calling `advance` does not itself free it.
- `SimHost::pending_command_count()` reports unsubmitted host staging. The
  default is zero, suitable for hosts that submit the entire command batch
  during `advance`. Custom staging hosts must implement the hook correctly.
  `PlayHost` and `RelayHost` implement it. Credits handed to a staging host are
  conservatively retained until its staging count reaches zero, even if it
  partially submits commands meanwhile.
- Host commands already pending at bridge construction are charged as well.
  Preexisting staging larger than the configured limit is grandfathered; new
  explicit commands are refused until enough staging is released. Construction
  does not delete preexisting accepted commands.
- One FIFO orders commands, debug edits, and controls. Queued command/debug
  messages have the data quota; queued timeline/step controls have an independent
  control quota. A full gameplay budget therefore does not prevent Play, Branch,
  or Step. A full control quota can still refuse another control.
- Queue-full returns `BridgeError::Backpressure` and accepts nothing. Replay
  refusal returns `BridgeError::ReplayReadOnly`. The by-value payload is consumed
  even on error, so retain the original or a clone for an intentional retry.
  Accepted gameplay commands are never coalesced or silently evicted.
- A terminal disconnect closes further admission and `is_alive()` becomes false.
  Acceptance means ordered staging, not guaranteed delivery after a failed
  session. Already staged data remains bounded until bridge destruction.

The mailbox quotas count queued messages; one message can additionally be in a
host call. Explicit gameplay permits cover that in-flight batch too. The sim
thread processes at most 64 reliable messages before checking the realtime tick
clock. Shutdown is out of band, wakes a waiting manual thread, and is checked
between messages and each tick in explicit multi-step batches. A single host
call cannot be interrupted.

## Held input and explicit steps

Held local input uses one coalesced latest-value slot. Automatic ticks sample the
newest admitted input. Both manual `Threaded::try_step(n)` / `step(n)` and
`control(ControlOp::Step(n))` capture the input when admitted: changing input
later cannot change the already queued batch. The next automatic tick resumes
sampling the latest slot; explicit stepping does not overwrite it.

Manual pacing waits for its admitted control/step to complete. Its single
mutable producer and separate control quota prevent gameplay saturation from
blocking a manual step. `try_step` exposes disconnection; the existing `step`
convenience method retains its no-return API.

## Replay viewer

A Viewer cannot accept live gameplay input or commands. Local bridge calls return
`ReplayReadOnly` without retaining the attempted data. The simulation does not
invoke input-to-command derivation or nonlocal bots during playback. The direct
`PlaySession::push_command` API returns `PlayError::ReadOnly`; its legacy
`set_input` API ignores Viewer writes. Recorded playback, play/pause, speed, seek,
and explicit stepping remain available.

Branch makes a PlaySession writable. Threaded admission observes successful FIFO
admission of Branch, so an immediately following command/input is accepted even
before the published snapshot changes. Only hosts advertising the explicit
`branch_enables_commands()` guarantee get this optimistic admission. Unsupported
or refused Branch requests cannot open admission. Actual host writability is
reconciled after each call without erasing later queued guaranteed branches. If a
custom host becomes read-only, already accepted explicit commands remain staged
until it reopens; new writes are refused.

ERP `sim.input` and `sim.command` report read-only refusal. `RemoteBridge` still
has its asynchronous RPC contract: `Ok` means queued, with server failures
available through `take_errors()`. Its input deduplication cache is invalidated
on RPC rejection and branch transitions so a rejected Viewer input cannot
suppress the same input sent after Branch.

## Deliberate limits

This is a count bound for newly admitted explicit commands and the local reliable
mailbox, not a byte bound or a whole-engine memory bound. Generic commands and
debug edits may own arbitrary allocations. Callback-derived commands, recorded
debug edits, submitted replay/session history, network retransmission copies,
RemoteBridge RPC/transport/error queues, and FFI batches are outside this bound.
No simulation golden values or recording format changed.

## Local verification (2026-10-02)

- `cargo test -p orr_bridge -p orr_session -p orr_remote --tests -- --skip measure_1000_body_scene`:
  212 passed, zero failed/ignored, one explicitly filtered performance case.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `git diff --check`: passed.
- The initial unoptimized run of `measure_1000_body_scene` produced 17 frames in
  three seconds, below its unchanged `>100` throughput requirement. That test's
  instructions use `--release`. The final-source targeted command
  `cargo test -p orr_remote --release --test measure measure_1000_body_scene -- --exact --nocapture`
  passed (one test, 181 frames in three seconds; the threshold remains `>100`).

Focused coverage includes paused budgets, reliable FIFO/control isolation,
custom host retention and read-only transitions, real RelayHost no-submit/submit
and disconnect staging, queued Branch with newer input, derivation suppression,
explicit-step input capture, saturated shutdown, a huge-step shutdown, realtime
clock fairness, and remote input-cache invalidation. Existing checksum/replay
regressions passed without golden updates. Full release/cross-platform CI remains
separate from these local checks.
