# Authored Room presentation camera (first bounded slice)

A schema-2 `room-escape-v1` entry can name a root-level `camera` JSON sidecar.
This optional profile is owned by the Room presentation consumer, not the
simulation. It does not change Frame bytes, checksums, replay/build identity,
physics, movement, or model transforms. Camera bytes do affect export content
identity. Arena/Collect entries cannot declare it.

`room-escape-3d-v1` generated projects explicitly include `room.camera.json`.
Projects with no camera field preserve the previous defaults: native Room uses
perspective distance 60, headless capture uses the old orthographic distance 50
and portrait-fit half-height 24, and the editor uses its old perspective view.
There is no silent retrofit of older project files.

## Schema

The strict JSON document requires `schema: 1`, `target: [x,y,z]`, `yaw`, `pitch`,
`distance`, and `projection`. Angles use radians except perspective vertical FOV
which uses degrees. Unknown, duplicate, null, or missing fields are errors.

- Target components: finite -64 through 64
- Yaw: finite -pi through pi; pitch: finite -1.4 through 1.4
- Distance: finite 1 through 128
- Projection: `{ "type": "perspective", "fov_y_degrees": 20..100 }` or
  `{ "type": "orthographic", "half_height": 1..64 }`
- Orthographic half-height fits the shorter viewport axis. Actual vertical
  half-height is `half_height * session_distance / authored_distance / min(aspect,1)`
- Dimensions are positive and at most 16384 each; exact aspect is retained
- Maximum document size is 4096 bytes

The same resolved camera is used for authored native runtime, capture, editor
rendering, picking, and overlays. Matrices and rays must remain finite. Manual
orbit wraps yaw and clamps pitch; pan uses the actual projection extent and
clamps target; wheel zoom saturates distance. Non-finite gestures are ignored.
Navigation changes are session-only and never silently persisted. Fixed camera
clipping planes are appropriate for this bounded Room profile; this is not a
general large-world camera rig.

## Authoring and lifecycle

The Room camera panel stages candidate values until **Apply camera**. Accepted
changes and **Undo camera** reset the live preview to the applied profile. The
panel has its own bounded history, separate from scene transactions. Invalid
Apply leaves the profile, history, and live preview intact. Controls are disabled
while playing or when the local scene snapshot is incoherent.

**Save camera** replaces only the camera sidecar. It does not save scene/model
edits or change the manifest. It pins the project directory, compares the exact
admitted manifest and last-saved camera bytes, rejects changed/nonregular/symlink
paths, writes and syncs a same-directory temporary file, atomically replaces the
camera file, and syncs the directory on Unix. Failure after replacement but during
directory sync is reported as uncertain durability, with the saved baseline
updated to the bytes actually replaced. This is bounded ordinary filesystem
validation, not hostile concurrent-filesystem isolation or a multi-file transaction.

Runtime Restart restores the admitted initial Frame and authored camera without
reopening files. Editor Restart room restores the currently applied camera,
including unsaved camera Apply changes. Opening a scene (even the same path)
clears project camera admission; reopen the complete project to reload its
sidecar. Successful Save As to a different scene clears camera admission;
failed Open/Save As preserves it. Authored profiles cannot reactivate merely by
returning to an old scene path.

If camera projection or GPU rendering fails, the last successful image can stay
visible with an error, but picking and overlays are disabled for that failed
presentation. No new-size camera is used to pick retained old-size pixels.

## Verification scope

Focused acceptance covers strict schema and bounds, aspect extremes, projection-
aware gestures, actual editor Apply/Undo/Save/reopen widgets, scene lifecycle,
Frame identity preservation, explicit software-GPU render/pick, and exact
production-tool source-hidden read-only export. OS-window interaction, physical
GPU, Windows local, camera follow, cutscenes, a generic rig system, and richer HUD
are separate work. This slice does not complete roadmap #94 or #104.
