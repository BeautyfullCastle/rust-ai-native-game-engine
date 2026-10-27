# Registered gameplay through ERP

ERP means Engine Remote Protocol (`crates/orr_remote/src/lib.rs`).
AI production and validation must use structured APIs by default. Computer use
is supplementary QA of the human interface, not a prerequisite for playing a
registered game or inspecting its output.

## Existing paths

`PreparedRegisteredProject::open` and `Editor::open_registered_project` select a
compiled registration from a bounded project manifest. The starter application's
`editor --project` is the equivalent startup path. Neither loads arbitrary code.
Each game supplies its registration, typed host, input codec and presenter; the
editor does not add another game-specific enum branch.

The existing ERP registry and world methods expose reflected types, stable entity
GUIDs and component values. Scene edits, transactions and undo/redo use the same
document and history as human edits. Existing `sim.*` methods expose the play
session, while managed input owns player leases and input sequences.

`activity.list` is a bounded ring query with `since`, `limit` and `include_reads`;
`watch.activity` supplies subscription updates. It is an activity record, not a
complete process log or a general secret-redaction service. Callers must inspect
truncation and retain their cursor. New gameplay summaries must never include
input payloads, tokens or lease credentials.

## Semantic gameplay bridge

The host-paired bridge is an asynchronous mailbox into the existing editor
command owner. A host request must not wait for that owner: the editor itself
uses ERP, so synchronous cross-calls would deadlock. Admission and completion are
separate observations. Human intents and AI intents pass through the same codec,
lease, sequence, frame and consumption checks.
Trusted native startup installs the pair before clients connect. Duplicate or
late installation fails closed; an installation timeout has an uncertain outcome.

The server attaches the actual connection and capability context. Caller-supplied
identity fields cannot grant authority. Read-only clients remain read-only;
control and submission require the existing simulation-control capability.
Bindings and request keys are connection scoped. Exact retries may recover a
retained result, but conflicting payloads, stale bindings and exhausted retained
history must not silently execute another command. Cancellation cannot undo an
input already consumed by the game. Unknown or uncertain results require
reconciliation, not a blind retry.
An authoring ACK does not settle editor observations; a stale-scope rejection
requires fresh inspection before another mutation.

`editor.gameplay.describe` returns the current `binding`, connection observation
token, `scope`, readiness and optional intent schema. Control/submit/cancel require
those three current fences plus a monotonically increasing decimal-string
`request_key`. `editor.gameplay.control` accepts play, pause, step, stop, claim or
release; its `queued` response is not lifecycle completion. Submit accepts an
intent ID and bounded payload. `editor.gameplay.status` takes the original
connection's command ID and reports pending, consumed, completed, rejected,
cancelled or uncertain. Reconnect requires fresh discovery; old fences are not
credentials and cannot be reused. Mailboxes retain at most 64 records, 16 pending
requests and 16 connections. Admission timeouts are at most five seconds;
expiration is observed at the next service boundary.

Physical window focus is human input context, not API authorization. Focus-only
cleanup must preserve a live, explicitly claimed API controller's existing lease
while human widgets remain disabled. Human/legacy focus cleanup, explicit Escape,
dialog/preview safety, lifecycle changes, disconnect, source errors and lease
timeouts retain their existing neutral/release behavior. An API connection merely
existing does not grant this exception.

## Rendering and acceptance

`view.screenshot` returns a bounded PNG of the settled editor framebuffer with
game/build identity, mode, paused state, tick, play-session epoch, checksum,
frame sequence and UI-frame metadata. Its epoch is not the view-stream timeline
epoch. A separate world query and screenshot are not an atomic observation
bundle; pause, compare the surrounding state and disclose that limitation.
After an edit, a late view update can return `view_stale` or `view_busy`.
Only this read-only capture may use bounded fresh-query retries while its
anchored paused state remains unchanged; this is not mutation retry permission.

The API acceptance path must open each of two registered project manifests,
query and edit reflected state, verify undo/redo, play, submit a semantic intent,
observe actual consumption and neutral completion, then pause, step and stop.
Stop must preserve the original editable document. A real WGPU framebuffer
capture must pass through the existing screenshot service and be decoded and
checked; a synthetic image is only a protocol fixture. Report adapter type and
separate GPU capture evidence from human window QA.

## Remaining boundary

Startup selection is currently an SDK/CLI API. The existing `HostSpec` listener
configuration can expose ERP with the existing authentication model; the starter
binary does not yet accept listener flags. In-process API acceptance does not
prove an external WebSocket deployment. There is no running cross-game
`project.open` RPC: a host is typed to one game, and `scene.load` cannot replace
that type. Such a switch needs an explicit host/client handoff, invalidation of
old bindings and capture requests, and a documented failure transaction. This
stage does not claim runtime discovery, hot reload, arbitrary plugin execution,
an atomic query/image bundle, or complete platform/CI coverage.
