# Nonblocking replay verification

`proposal.verify` and `verify.self` keep their existing delayed JSON-RPC response
and report shape. They do not expose a new job API. This applies to both real
WebSocket clients and in-process `LocalConnector` clients, including a
`LocalHost` with `listen: false`. Direct `call_local` calls remain synchronous.

## Admission and snapshot

Each `ErpServer` admits at most one verification. After authorization (and the
existing client-mode rejection), a second verification returns
`LIMIT_EXCEEDED` / `verify_busy` before copying frames or inputs, parsing checks,
or decoding a recording. There is no verification queue.

For `replay` and `last_play` inputs, an explicit top-level `ticks` above
`max_verify_ticks` returns `LIMIT_EXCEEDED` / `verify_limit` with the existing
`max` metadata during preparation, after checking the top-level options and
proposal identity but before copying input JSON, either frame, or `last_play`,
and before creating a worker. This refusal takes precedence over decoding or
validating replay content and looking up the last recording. The document,
proposal, revisions and history remain unchanged. Omitted or within-limit
recorded tick caps keep their existing behavior, including capping a longer
recording. `bot` and `idle` retain their separate `inputs.ticks` limit and
top-level tick clamping behavior.

Admission captures the current document's base and candidate frames together
with the proposal's `verified_state`, the type registry, game hooks, options,
checks, and input specification. `last_play` captures the recording that exists
at admission. Snapshot preparation still happens on the host thread; replay
base64 decoding, decompression, simulation, metrics, checks and report building
run on the worker. The worker never owns the live document or play session.

Edits, undo/redo, play control, normal queries and view publication may continue.
The report describes the captured state even if a proposal changes or is
rejected while verification runs. Completion never rereads the proposal or
recaptures its revision. `proposal.accept_verified` remains the concurrency
guard and refuses a changed document or proposal. A state stamp is not proof
that checks passed.

## Lifetime and failure

The server uses a standard thread independently of the network runtime. The
existing verification core can run its two sides in parallel: one coordinator
plus one scoped side thread, at most two execution threads per active job.

Disconnect and server drop set a cooperative cancellation flag. Cancellation is
checked before setup, around replay preparation, before simulation ticks,
between sequential sides, after each tick, and around report construction. No
partial successful report is returned. A canceled job occupies the slot until
the worker actually exits; reconnecting cannot create overlapping workers.

The worker holds an inbox wake sender, not the network state or a connection's
response sender. Completion wakes an idle host. Only the host reaps finished
workers, records terminal activity, accounts for errors, and sends the original
response once. Disconnected clients receive no response. Notifications still
execute but have no response ID. Drop detaches a running worker instead of
waiting for it, so a blocked hook cannot hold the host or its connections open.

Worker panics become `INTERNAL_ERROR` / `panic`. The live play session is never
dropped because isolated verification failed. Spawn failures also return an
error and leave the slot reusable.

## Explicit limits

The default 6000-tick limit bounds simulation execution, not decoded replay
memory, one tick's duration, or total elapsed time. Cancellation cannot preempt
a blocked custom hook, replay decoder, or individual simulation tick. Hard
preemption, absolute replay allocation limits, replay-format hardening and
general RPC/transport queue bounds are separate work. A client call timeout
alone does not disconnect its transport and therefore does not cancel a job.
Early refusal avoids admission copies for an already over-limit recorded
request; it does not bound upstream request parsing or preparation allocations
for accepted recordings.

No simulation arithmetic, recording format, or golden checksums change.

## Recorded admission regression coverage (2026-10-05)

Six normal-input unit tests use a three-tick recording made by `PlayController`
and an existing host limit of two ticks. Before the admission change, all four
`replay` / `last_play` and self / proposal refusal cases failed because
preparation succeeded. Afterward they reject at preparation with the unchanged
error contract. The compatibility tests preserve complete reports, checksums,
state stamps and document/proposal state for omitted and within-limit caps,
including the existing zero-to-one clamp, and preserve scripted top-level
clamping. No malformed recording or panic fixture was added.

The release `orr_remote` library suite passes all 80 tests, the existing
`nonblocking_verify` and `proposals` integration suites pass 9 and 13 tests,
and release library Clippy passes with `-D warnings` on Rust 1.97.1. This is
focused regression coverage, not a full workspace or cross-platform validation
claim.

## Verification of this slice (2026-10-02)

- Nine normal-input concurrency/lifecycle tests pass through `LocalConnector`
  and real WebSockets, including read/edit/control and real-time view progress
  while a verifier is held, busy admission, cancellation/slot reuse, captured
  proposal and last-play state, idle completion wake, prompt host shutdown,
  notification routing, and synchronous `call_local`. The suite also passed
  five consecutive stability runs.
- Core cancellation, sequential/parallel report equivalence, ordinary recorded
  replay equivalence, and selected CLI/MCP/editor verification flows were
  rechecked after the final test changes: 11 focused tests passed.
- Earlier focused suites passed 102 tests: edit verification 19, remote
  activity/client-mode/local/proposals 30, generated guide 4, editor 19, CLI 20,
  and MCP 10. Together with the nine added transport tests, this is 111 unique
  focused tests; repeated runs are not additional unique coverage.
- `cargo clippy --workspace --all-targets -- -D warnings` and `git diff --check`
  passed. Independent source review found no blocking issue.

The new malformed/problem-input regression is deferred and excluded from this
change set. Existing tests are preserved. The panic-containment implementation
has source review, but this slice does not claim new problem-input runtime
coverage. No full workspace test/release run or cross-platform CI result is
claimed here.
