# Optional Room character clips

`room-character` is default-off in `orr_sample` and `orr_editor`. It adds one
presentation-only skinned player to the closed RoomEscapeV1 creation workflow.
The original CC0 Key Courier fixture has a head, visor, torso, articulated arms,
legs and boots, six joints and three distinct looping clips.

## Create, author, play and export

Build the creator with `room-character,project-create`, then use:

```
orr_new_arena --output /absolute/new-project --template room-escape-character-3d-v1 --seed courier
```

Open it in an editor built with `room-character` using `--room-project PATH`.
The Room character panel assigns searching, carrying-key and escaped clips and
per-state playback speeds (1/4x, 1/2x, 1x, 2x or 4x), and optional crossfade
duration in simulation ticks (0..120).
Apply validates all three clips and the complete reachable presentation before
changing either in-memory document. Undo/Redo character edit coordinates the
model binding's default searching clip with the character map and speed choices.
Use **Apply character settings**, then **Save
character and model bindings** for a changed mapping. Other model edits retain
the existing verified package/asset, local transform and clip admission gates.

Run `room_escape --project PATH` built with `room-character`. Export using
`orr_export_room` built with `room-character,project-export` and a trusted
prebuilt `room_escape` with the same capability. Export owns the exact character,
model and scene documents plus their installed package closure; source package
paths, generator execution and HOME are not runtime dependencies.

The old Room and Room UI templates remain unchanged. Other consuming routes do
not acquire Room character support merely because Cargo unified an animation
dependency. `PreparedProject::open` and `open_with_options` remain narrower;
explicit consumers use `open_with_capabilities(..., true)`.

## Data and time contract

`entry.character` names a distinct root-level, non-hidden document, for example
`room.character.json`:

```json
{"schema":1,"player":"e_0123456789abcdef0123456789abcdef","searching":0,"carrying":1,"escaped":2}
```

Unknown/duplicate fields, sequence/null forms, noncanonical GUIDs, repeated or
out-of-range clips, wrong actors, extra animated bindings and missing verified
assets fail closed. Exactly the declared PLAYER is animated; other bound actors
are static. The v2 model sidecar remains the sole owner of package identity,
source hash and local TRS. Its default descriptor equals searching + Loop.

The editor rechecks the installed declaration and the current generation-bearing
PLAYER role before sampling; same-path scene replacement cannot reuse a mapping
on a different role or silently fall back to generic Yard animation.

Selection reads the same authoritative Frame as body placement:

1. `RoomRun.won != 0`: escaped
2. Otherwise `RoomRun.key_collected != 0`: carrying
3. Otherwise: searching

Schema 1 keeps exactly its original fields and fixed 1x behavior; it rejects a
`speeds` field. Schema 2 requires a complete object of explicit speed strings:

```json
{"schema":2,"player":"e_0123456789abcdef0123456789abcdef","searching":0,"carrying":1,"escaped":2,"speeds":{"searching":"1/4x","carrying":"2x","escaped":"4x"}}
```

Only `"1/4x"`, `"1/2x"`, `"1x"`, `"2x"` and `"4x"` are accepted. Missing,
duplicate or extra speed keys, nulls, arrays, numbers and enum-shaped objects
are invalid. Generated templates stay schema 1. Changing a speed in the editor
upgrades the candidate to schema 2; Apply/Undo/Redo/Save preserve the whole
document, including the schema choice. Selecting all 1x in schema 2 is valid.

Every clip uses the absolute Frame tick / tick rate multiplied by its state's
exact rational speed (fixed 1x in schema 1). The shared sampler reduces the
dyadic period with bounded integer modular arithmetic before conversion to
presentation floats, retaining adjacent-tick phase even near `u64::MAX`.
Transitions do not reset phase. Schemas 1/2 snap directly to the selected clip;
Edit/Stop requests rest, and Play or Seek at tick zero samples clip time zero.

Schema 3 requires both the same complete speeds object and `crossfade_ticks`,
an integer from 0 to 120. The editor upgrades a candidate to schema 3 when the
crossfade duration changes. Zero preserves snap behavior and the legacy storage
budget. Positive duration blends from the last observed visible pose to the
incoming absolute-phase pose using local translation/scale lerp and shortest-path
normalized quaternion slerp. Hierarchy, palettes and deformation are revalidated.

Native and headless/export consumers observe every authoritative simulation tick,
including ticks between draws. Editor presentation observes coherent snapshots;
a missing snapshot gap snaps because the transition history is unknown. A new
state during a blend freezes its current blended pose as the next outgoing pose.
Repeated paused draws do not advance or resample transition history. Explicit
seek (including same-tick local editor seek), restart, rewind, source revision,
entity generation, document or tick-rate changes reset presentation history.
There is no animation clock in ECS or modification to checkpoints, inputs,
replays or golden checksums. See [crossfade contract](room-character-crossfade.md).

## Limits and failure handling

Room keeps 68 bindings, eight unique verified assets, 64 MiB aggregate decoded
storage and the existing shared imported-draw bound. Animated storage includes
vertices, weights, hierarchy, palettes and every authored clip/key, plus pose
working storage. Legacy and zero-duration paths retain the existing two-buffer
charge; positive crossfades charge three additional buffers before admission
(five peak buffers total, at most two retained). Static and skinned geometry
share one depth pass; an imported
player replaces its proxy and the collected key is hidden in both paths.

The editor pair save preflights both current files and the manifest, prepares
both replacements and a rollback file, and restores the original model file
when the second replacement fails normally. Saved baselines advance only after
both replacements. Directory-sync failures report uncertain durability. This
is not a filesystem transaction across crashes or hostile races; simultaneous
external changes require reopening rather than overwriting.

## Acceptance attribution

The runtime headless CLI accepts one held input. Exported production-binary
checks cover searching/carrying and restart. The full fresh-interaction
searching → carrying → escaped → restart sequence uses the production native
App's input-step/restart methods in an explicit source-hidden, read-only exported
project child test. This is an App harness, not a claim of a native-window
interaction or an exported CLI second-press feature. The real editor harness
uses control acquisition and fresh keyboard releases/presses for key and exit.
GPU claims, when verified, are software-adapter claims only.

Out of scope: locomotion blending, controllers, root motion, IK, retargeting,
animation graphs, new importers, new shadow features and physical-GPU coverage.
