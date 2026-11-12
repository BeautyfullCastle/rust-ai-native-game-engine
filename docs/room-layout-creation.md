# Create an authored Room puzzle

On Linux, build the existing `orr_new_arena` creator with `project-create,room-project`.
Use `--template room-escape-3d-v1 --seed puzzle --output /absolute/new-project
--room-layout /path/layout.json`. The output directory must not exist. Without the
option, existing starter bytes and behavior are unchanged. Character/Collect/Arena
templates reject this option.

The strict JSON recipe requires all five fields:

```json
{"schema":1,"player":["-3","0"],"key":["-2","0"],"exit":["3","0"],
 "walls":[{"center":["0","0"],"half_extents":["0.25","1"]}]}
```

Pairs are X/Z. Coordinates are plain decimal strings, never JSON floating-point
numbers: optional minus, digits, optional dot followed by digits, at most 48 bytes.
The engine's integer-only parser rounds to Q48.16. Actors keep canonical Y=0.5;
walls have center Y=0.5 and half-height 1. The wall array replaces all starter
walls and may be empty or contain at most 64 boxes. No authored floor is added.
Existing Room admission enforces bounds, canonical geometry and initial player
clearance. Key/exit overlap or puzzle solvability has no additional rule.

Unknown/duplicate/missing fields, nonregular files and recipe-leaf symlinks are
rejected. Input is captured once, at most 64 KiB. The captured recipe is applied
only in the creator's private stage, with complete scene/project admission before
atomic no-replace publication. Failure does not publish a new project or change
an existing destination. Ordinary filesystem errors are covered, not hostile
filesystem races or power-loss durability.

The project keeps the starter's static art, installed packages, camera and actor
GUIDs. Ordered walls have seed-derived deterministic GUIDs. The creator reports
the actual saved entity GUIDs and admitted initial checksum. Same seed, recipe
and tool reproduce project bytes. The recipe is not retained as a live link:
edit the generated scene with the existing editor, or create a new project.

## Check the playable result

Use `orr_playtest_room --project PATH --script route.json --json` with
`room-playtest`. For the example, interact once to collect the nearby key,
release interaction while moving Z+ for 40 ticks, X+ for 120, Z− for 40, then
interact for one tick. The detour wins; attempting X+ for 120 ticks directly is
blocked by the authored wall and cannot win. Each movement step expects key=true,
won=false; the final interaction expects both true. See [script format](room-playtest.md).

This is scripted CPU gameplay validation, not a solver, natural-language success
measurement, physical input, window/GPU or export acceptance. Admission alone
does not prove a puzzle is completable. Always test the authored route.
