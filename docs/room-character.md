# Optional Room character animation

`room-character` is default-off in `orr_sample` and `orr_editor`. It adds one presentation-only skinned PLAYER to RoomEscapeV1. The CC0 Key Courier fixture has six joints and three named clips.

## Create, author, save and run

Build the creator with `room-character,project-create`:

```
orr_new_arena --output /absolute/new-project --template room-escape-character-3d-v1 --seed courier
```

Open an editor built with `room-character` using `--room-project PATH`. The Room character panel selects searching, carrying-key and escaped clips, per-state speeds (1/4x, 1/2x, 1x, 2x, 4x), bounded crossfade ticks and **Play escaped clip once**. Selectors show admitted index, name and duration; duplicate names retain indices and blank names have a fallback. Stale metadata disables editing; unavailable draft indices remain explicit.

Apply validates every mapped clip and reachable presentation before changing either in-memory document. Character Undo/Redo coordinates the model binding's searching default. Use **Save character and model bindings**. Unapplied choices count as unsaved work and block paired Save/package/export; Apply or Reset character draft first. Reset preserves applied unsaved edits/history.

Run `room_escape --project PATH` built with `room-character`. Export through `orr_export_room` built with `room-character,project-export`, using a trusted runtime with that capability. Export owns exact character/model/scene documents and their installed package closure; source paths, generator and HOME are not runtime dependencies. Plain Room/Yard consumers require explicit character capability rather than Cargo feature unification.

## Document and timing contract

`entry.character` names a distinct root-level non-hidden document:

```json
{"schema":1,"player":"e_0123456789abcdef0123456789abcdef","searching":0,"carrying":1,"escaped":2}
```

Unknown/duplicate fields, sequence/null objects, noncanonical GUIDs, repeated/out-of-range clips, wrong actors, extra animated bindings and missing verified assets fail closed. The exact declared sole PLAYER is animated; other bindings are static. The v2 model sidecar owns verified package/hash/local TRS and defaults to searching + Loop. Current source lifetime and generation-bearing PLAYER role are rechecked.

State comes from the same immutable Frame as placement: won takes precedence, then collected key, then searching.

- Schema 1 retains exact original fields and 1x looping.
- Schema 2 requires complete strict `speeds`, such as `{"searching":"1/4x","carrying":"2x","escaped":"4x"}`.
- Schema 3 additionally requires `crossfade_ticks` in 0..120.
- Schema 4 uses exactly schema 3's fields and opts into escaped one-shot playback. The checkbox upgrades to 4; disabling returns to 3. Changing speed/crossfade preserves enabled one-shot policy. The creator still emits schema 1.

Schemas 1..3 keep absolute tick/rate looping. Schema 4 keeps searching/carrying looping, but escaped starts at time zero on its first observed win tick, advances at the escaped speed and clamps to the final pose. Crossfade blends toward this entry-relative target. Repeated draws do not advance time; forward gaps in a known escaped state retain entry. First observation after an unknown transition, backward time, seek/restart/source/document/model/context replacement starts a fresh observed entry; it does not reconstruct an unseen event tick. Native playback observes simulation ticks; editor sampling observes received snapshots. Stateless escaped sampling rejects without an observed cursor, rather than inventing an entry time.

Edit/Stop shows rest; Play/Seek zero samples clip zero time. No clock is written into ECS, checkpoints or replay. Crossfade retains bounded poses and snaps on discontinuity.

## Limits and failures

Room retains 68 bindings, eight unique assets, 64 MiB aggregate decoded storage and the shared draw bound. Admission charges all clips and bounded crossfade pose work. Static/skinned geometry shares depth; imported PLAYER replaces its proxy and collected KEY is hidden.

Paired Save preflights both files and manifest, prepares replacements/rollback, and restores the model file after ordinary second-file failure. Baselines advance after both replacements; directory-sync errors report uncertain durability. This is not a crash-safe multi-file transaction; external changes require reopen.

## Verification scope

Focused CPU tests cover strict/legacy documents, late victory start/hold/rates/crossfade/discontinuities, actual editor keyboard interaction, Save/reopen/history, and source-hidden exported game simulation through key pickup and victory. Exported CLI startup checks model poses; its held-input option does not synthesize a second interaction press. These checks do not establish native-window/GPU/audio coverage. Existing Windows positive publication access errors remain unresolved and are not retried. Locomotion, root motion, IK, retargeting and general animation graphs remain outside this feature.
