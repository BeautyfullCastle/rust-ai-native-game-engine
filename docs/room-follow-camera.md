# Optional Room PLAYER-follow camera

This bounded addition extends the existing optional `room-project` presentation
camera. It adds no dependency, simulation component, input, replay state, or
Frame mutation. Existing generated projects and schema-1 documents remain fixed
cameras. Camera content affects export content identity, never gameplay identity.

## Authored contract

A schema-2 camera has all the schema-1 fields (`target`, `yaw`, `pitch`, `distance`,
`projection`) and one required object:

```json
"follow": {
  "player": "e_0123456789abcdef0123456789abcdef",
  "offset": [0.0, 1.0, 0.0]
}
```

The GUID must be canonical and identify the exact sole authoritative Room PLAYER.
The three world-space offset components are finite and in [-16, 16]. Schema 1
forbids `follow`; schema 2 requires it. Null, arrays in place of objects, duplicate
or unknown fields, missing members, unsupported versions, malformed GUIDs and
out-of-range values are errors. The existing 4096-byte limit and pose, projection,
viewport and finite-matrix bounds remain unchanged.

Every rendered frame resolves the GUID through a coherent scene index, checks
its exact entity generation and reverse mapping, verifies the sole PLAYER role
and its Body, then derives the look-at target as Body position plus offset.
Body coordinates are checked in fixed point against the existing Room bounds
before float conversion. No prior frame, elapsed time, target rotation, smoothing,
velocity estimate, spring or interpolation state contributes to the camera pose.
The same immutable-frame resolver serves editor, standalone and headless export
captures. Identical frame, authored document, session angles/distance and viewport
produce the same camera, including after backward Seek and checkpoint restoration.
Missing or reassigned targets reject the presentation rather than following a
recycled entity or silently choosing another PLAYER.

## Authoring and navigation

Open **Room camera**, enable **Follow PLAYER**, and edit **Follow offset X/Y/Z**.
The toggle selects the current sole PLAYER GUID, not the selected model binding.
The other authored fields are retained, including the fixed `target`, so disabling
follow restores a schema-1 fixed profile without losing its framing. **Apply
camera**, **Undo camera edit**, **Redo camera edit** and **Reset camera defaults**
reset the transient live orbit to the accepted document. Invalid changes leave
the applied document and history intact. Authoring is disabled during Play,
preview or an incoherent snapshot.

Mouse orbit and wheel zoom remain session-only, rotating/zooming around the live
PLAYER anchor. Panning is explicitly disabled in follow mode. Reset restores the
authored yaw, pitch and distance; it does not detach from the PLAYER. Fixed-mode
orbit, pan and zoom keep their prior behavior. Navigation never persists itself.

**Save camera** replaces only the camera sidecar. Scene, model, material and
character documents and installed assets are untouched. The existing exact-byte
manifest/sidecar and pinned-directory checks remain in force; this is ordinary
filesystem race detection, not a hostile-filesystem isolation guarantee.
The production panel also pins the editor source identity and revalidates the
live target before Apply, history and Save. A successful scene replacement or
host restart retires old panel drafts, including A-to-B-to-A changes; reopen the
complete project before saving them. Failed source operations and read-only sync
are not source replacements.

Play, Stop, Seek and **Restart room** derive the anchor from their current frame.
Restart room restores the authored session camera and initial gameplay frame.
First editor startup consumes the already admitted Room scene bytes. As with
Arena and Collect, later host reconnect/restart reads the saved scene path.
A host reconnect/restart that reloads a now-incompatible target refuses the
restart with a diagnostic and preserves the previous editor/camera; it never
silently downgrades follow to fixed. Explicit scene Open continues to retire
project camera admission, as before.

## Boundaries and verification

This is world-space target following for one Room PLAYER, not a general camera
rig, target selector, collision/occlusion camera, cutscene system or damped chase
camera. Lens, clipping, orthographic portrait fit and perspective behavior reuse
the existing camera implementation. Static models, per-instance material factors,
character clips and checkpoint state remain independent consumers.

The dedicated feature workflow requires strict schema/admission and editor
widget/lifecycle tests, default-feature boundaries, static and character/material
profiles, explicit GPU main-viewport readback, and production source-hidden
read-only export parity for static/material and character-PLAYER/static-material
profiles at initial, movement and key-pickup states. The editor GPU workflow
also reaches the exit/won state; the existing constant-input export interface
cannot produce a second interaction edge, so won-state export parity is not
claimed. A skipped/ignored GPU test is not acceptance. Local
software GPU, hosted mandatory CI and development-union CI are separate receipts;
physical GPU, native-window interaction and Windows-local execution are not
implied. This slice does not close roadmap #94/#98 or change the original roadmap
completion denominator.

Design comparison: Unity's [Cinemachine Follow documentation](https://docs.unity3d.com/Packages/com.unity.cinemachine@3.1/manual/CinemachineFollow.html)
distinguishes world-space offset from target-relative modes and damping. Orrery
uses the narrow world-space anchor concept and deliberately omits stateful damping
so authoritative frame seeking remains exact.
