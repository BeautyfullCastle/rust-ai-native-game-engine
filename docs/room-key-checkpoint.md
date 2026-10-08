# Room key checkpoint v1

This optional Linux standalone consumer saves one key-collected milestone. It
never serializes a Frame, arbitrary position, physics cache, input history, camera,
HUD, win state or inventory. Resume recreates the admitted authored initial scene
at its validated player spawn and restores only `RoomRun.key_collected = 1`.
Core simulation crates and their deterministic checksums are unchanged.

## Author a checkpoint game

Build tools with `orr_sample/room-checkpoint,orr_sample/project-create` (and
`orr_sample/project-export` for export). The feature is default-off and includes
Room authored UI support. Choose a new canonical lowercase UUIDv4 for an
independent game; relocating or exporting an existing game retains its UUID.

```
orr_new_arena --output /absolute/new-project \
  --template room-escape-ui-3d-v1 --seed my-room \
  --room-checkpoint 12345678-1234-4234-9234-123456789abc
```

The sample UUID is for examples, not a globally unique identity for your game.
The creator requires the UI template and explicit UUID; it does not infer or
silently generate an identity. In an editor built with `room-checkpoint`, open a
Room UI project, expand **Room checkpoint**, enter its UUID, and select **Enable
checkpoint**, then **Save checkpoint**. Undo edits in that panel, then Save to
persist the undo. Play makes these authoring controls read-only. Manifest saves
leave scene, model, camera and HUD bytes unchanged. Reopen after saving checkpoint
metadata before using an already-open camera/HUD panel to save those documents.

Schema 1/2 projects keep their existing behavior. Schema 3 remains exclusively
Collect's high-score identity. Room checkpoint projects use schema 4:

```
"progress": {
  "schema": 1,
  "game_id": "12345678-1234-4234-9234-123456789abc",
  "profile": "room-key-checkpoint-v1"
}
```

The schema requires a RoomEscape entry with models and authored Room UI. Consumers
without declared checkpoint support reject this metadata rather than ignoring it.
Disabling the checkpoint explicitly returns the manifest to schema 2; it does not
delete any player profile files.

## Player choices

- **Resume** restores the saved key at the authored spawn; it is available only
  when a matching valid checkpoint was read
- **New Game** durably writes an empty checkpoint before replacing the live game
- **Restart session** restarts at the initial scene without clearing the saved key
- **Continue current session** leaves the current session intact

Acquisition is observed immediately after every authoritative App step, rather
than through potentially dropped presentation snapshots. A key save is attempted
once per standalone session or explicit New Game cycle. Restarts/resumes clear held input and require a neutral
sample before another interaction. Win state is never restored or separately
saved. Editor Play, admission, replay/session construction, headless execution,
capture and export smoke do not open the player profile.

## Identity and storage

The namespace is the explicit UUID plus a SHA-256 challenge digest over versioned
gameplay rules, seed/tick rate, admitted actor roles, body/collider properties and
spawn. Records use fixed-width little-endian fields and sorted semantic records,
so entity GUID order is irrelevant. Names, paths, YAML formatting, camera, UI and
model presentation are excluded. Relevant gameplay changes select a new namespace
instead of trying to migrate an old checkpoint.

Data is external to the project and executable/export tree:

```
$XDG_DATA_HOME/orrery/games/<uuid>/room-key-checkpoint-v1/<challenge>/checkpoint.json
```

When `XDG_DATA_HOME` is absent or relative, the absolute HOME fallback is
`$HOME/.local/share`. No valid absolute base means persistence is unavailable.
The closed JSON envelope is at most 4096 bytes and contains only schema, profile,
UUID, challenge and a boolean key milestone. Unknown, future, mismatched or corrupt
bytes remain untouched and put the session in read-only mode. New Game does not
silently repair those files.

A nonblocking exclusive advisory lock is held for the session. Concurrent
contenders can read an existing valid checkpoint but never become writers later.
Read-only directories/files also allow Resume when their content is valid. The
store rejects symlinks, FIFOs, unexpected ownership/permissions, hardlinked files,
unsafe path components, and changed path/lock/file identities. The lock protects
cooperating processes; this is not a guarantee against a hostile same-user process
racing the final publication check.

Writes use bounded exclusive staging, file fsync, atomic rename and directory
fsync. Failure before publication preserves prior bytes. Failure after publication
reports **durability uncertain**, disables further writes in that session and does
not claim a durable reset. The UI leaves the current game intact after an unproven
New Game reset. Missing profile directories and the lock may be created only by
normal standalone checkpoint-session startup.

## Verification gates

```
tools/check-room-checkpoint.sh contracts
tools/check-room-checkpoint.sh gpu-export
tools/check-room-checkpoint.sh syscall
```

Each gate requires positive test counts. GPU/export uses copied compiler-selected
production binaries and App test executable, a source-hidden namespace, a
read-only exported bundle, an external profile directory, four fresh App processes,
and before/after bundle hashes. It exercises the actual App host and offscreen HUD;
it is not native-window or physical-GPU evidence. The real exported runtime also
runs its headless/capture smoke without profile writes. The syscall gate separately
requires successful tracing with positive checkpoint-write syscalls and no profile
calls in headless/capture smoke. A ptrace restriction is a blocked gate, not a pass.
