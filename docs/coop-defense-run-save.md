# Stormglass Run Save v1

The optional web game can download and restore a bounded defense run. This is
an input replay, not an engine Frame checkpoint or a cross-version migration.
No Rust/core state format changes are involved.

## Use

Open **진행 중 방어전 저장 / 복원 · Run Save v1**, then download a run as
`stormglass-run-v1.json`. Select that file with the separate run-file control to
restore. Scene YAML import still creates a new authored game; it is not a run
restore. Files remain local and are not uploaded.

A successful restore is paused, with no pending inputs or board selection.
Choose **계속** to continue. Terminal runs remain terminal. A failed restore
retains the live game, its recorded history, queued inputs and sequence counter.
Selecting a different file, loading a scene or restarting invalidates an
outstanding restore. Only one candidate replay runs at a time; if another file
is selected during validation, select it again after cancellation completes.

## Contract

The JSON envelope has exactly `format: "orr.stormglass-run"`, `version: 1`,
`wasmSha256`, `initial`, `steps`, `sequence` and `checksum`. The SHA-256 must
match the exact WASM bytes already loaded by the page. Imported files cannot
choose executable code, WASM or URLs.

`initial` is either `{kind: "seed", seed: <u32>}` or
`{kind: "scene", base64: <canonical base64 of original UTF-8 YAML bytes>}`.
Authored scene seeds are not overridden. Each step is
`[action, sourceCell, targetCell, sequence]`; neutral steps are `[0,0,0,0]`.
Every executed step is recorded, including refused commands and Restart.
Restart retains history because it is not a fresh engine initialization.
The separate sequence high-water includes inputs queued but subsequently
cleared; the queue itself is deliberately not saved.

Limits are 2 MiB per file, 36,000 steps and 64 KiB of authored scene bytes.
Numbers are finite bounded integers; action, cell, sequence, checksum, hash,
UTF-8 and base64 constraints are validated. Duplicate/unknown fields and
nesting beyond eight levels are rejected. Command IDs never wrap past u32.
Exceeding the recording cap disables saving explicitly while gameplay
continues; importing a new scene starts a fresh recording.

Restore instantiates a separate copy of the retained WASM, initializes and
replays it, compares the 16-hex-digit checksum, then commits synchronously.
Presentation history and both event adapters reset after the swap. Replay has
a cooperative five-second deadline, yielding after 64 steps or eight
milliseconds. This is **not a hard timeout**: one synchronous WASM call cannot
be preempted.

## Acceptance and limits

Focused tests cover strict parsing, caps, sequence gaps, seeded and authored
replays, checksum and next-input equivalence, Restart history, and failed or
stale candidate preservation. The bundled-page harness exercises actual WASM
with mock DOM, Blob and File objects. That harness is not proof of a real
browser file picker, GPU, or mobile browser behavior. The same loaded WASM
requirement intentionally prevents incompatible saves from silently migrating.
