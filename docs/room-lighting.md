# Saved Room point lighting

The optional `room-lighting` feature adds one saved, presentation-only point light
for schema-2 `room-escape-v1` projects. It is disabled by default in `orr_sample`
and `orr_editor`. It does not alter Frame state, simulation, gameplay or checksums.

## Project contract

Add `"lighting": "room.lighting.json"` to the existing manifest's `entry` object.
Create the referenced root-level regular file:

```json
{"version":1,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12}}
```

The document is at most 4096 bytes. `point_light` is required; `null` disables the
light. Position is finite within ±1e9 world units, linear RGB and intensity are
0..=1e4, and range is 1e-6..=1e9. Unknown/duplicate fields, positional objects,
trailing values, invalid versions, null manifest paths, symlinks and aliases of
other project files are rejected. A lighting declaration requires object-shaped
manifest and entry; existing field order and non-lighting decoding are preserved.
Undeclared sidecars are ignored. Legacy `PreparedProject::open` remains an
unsupported consumer; actual lighting routes explicitly use
`open_with_lighting(root, true)`. A unified Cargo feature alone is not consent.

## Authoring and delivery

Open the complete project in an editor built with `room-lighting`. The Lighting
panel stages edits; Apply updates presentation, Undo/Redo retain up to 32 steps,
and Save replaces the sidecar. Reopen to load saved values. Editing requires a
coherent local Edit snapshot. Play, remote editing, stale/replaced sources and
changed on-disk manifest/sidecar bytes refuse authoring. Source replacement or a
successful restart retires existing panel drafts, even before their first frame.
Saving another scene path requires reopening the complete project.

A failed write before replacement leaves saved bytes and authoring history intact.
If replacement succeeds but directory syncing fails, the panel reports uncertain
durability and advances its own saved baseline; it does not report full success.
These checks detect ordinary changes, not hostile concurrent filesystem races.

The Room runtime and exporter built with `room-lighting` consume the declaration.
Exports retain exact lighting bytes. Rendering applies the light to imported
models through the existing renderer; this adds no shadow lights or gameplay
behaviour. Existing optional animated-model routes remain compatible, without
adding character-driven lighting.

## Acceptance

Run package tests, sample `room_lighting` and editor `room_lighting` tests with
`room-lighting,project-create`; run default-off admission separately. Editor panel
unit tests cover transactional failures and lifecycle guards. The ignored editor
GPU test requires `ORR_REQUIRE_GPU=1`. The ignored sample
`room_lighting_export` test additionally needs `project-export`, absolute
`ORR_ROOM_RUNTIME` / `ORR_ROOM_EXPORTER` paths and
`ORR_REQUIRE_PROJECT_ISOLATION=1`. It runs production binaries with source paths
hidden and exported files read-only, checks exact PNG/sidecar parity and unchanged
simulation. Missing GPU or isolation is a failure, not successful coverage.
Linux software Vulkan evidence does not certify hardware GPUs or other platforms.
