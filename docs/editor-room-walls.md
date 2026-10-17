# Editing Room walls in the editor

Room wall authoring is part of the Windows x86-64 editor feature set. Build or
run the editor with `windows-projects` enabled:

```powershell
cargo run -p orr_editor --features windows-projects
```

From the default local Physics editor, use **File → New Room project…** to
create a Room project. Open **File → Room level authoring…** and expand
**Manage existing Room walls** to inspect walls already in the scene. The
authoring window also has controls for creating a wall.

## Existing walls

Choose **Read Room walls**, select a listed wall, and choose **Read selected
Room wall** before editing. **Apply Room wall** can rename the wall and change
its X/Z center and horizontal half-extents. The wall remains a Room wall; its
vertical placement and other component fields are retained. Each accepted
update, duplicate, or removal is a scene edit in the editor's normal undo/redo
history.

**Duplicate Room wall** copies the selected wall's components, assigns a fresh
entity GUID and the requested name and X/Z position. It does not duplicate a
model-binding sidecar entry. To remove a wall, refresh/select it, check **I
confirm removing this Room wall**, then choose **Remove selected Room wall**.
Removal is rejected if the model-binding sidecar is not in sync or still binds
that wall; remove and save that binding through the model panel first. The
scene is checked against the admitted Room profile before the removal is
applied.

Save the scene with **File → Save**. Wall geometry is stored in the Room scene,
not in a separate wall file. Reopen the project to verify the saved result.
Room's play mode uses the authored colliders, so a wall can be checked against
the player and interaction flow before export.

## Scope and limits

This workflow edits the existing Room wall role: axis-aligned box colliders
with horizontal X/Z placement and dimensions. Walls must meet the Room
profile's full-player-height rule; its scene validator allows at most 64 walls,
centers within the admitted ±16 X/Z envelope, and box half-extents from 0.1 to
8. This is not a general-purpose level editor, mesh collider authoring tool,
terrain editor, or navigation authoring system. Model bindings and camera
settings remain separate project sidecars and are not silently rewritten by
wall operations.

## Windows UI acceptance

The opt-in integration test exercises the real editor UI and requires a
hardware Vulkan adapter. On Windows x86-64, run it with an empty destination
path whose parent already exists:

```powershell
$env:WGPU_BACKEND = 'vulkan'
$env:ORR_REQUIRE_GPU = '1'
$env:ORR_ROOM_WALL_ITERATION_OUTPUT = 'E:\Games\Room wall iteration'
cargo test -p orr_editor --features windows-projects --test room_wall_iteration_ui
```

The test covers create, list/read, update, invalid-edit refusal, duplicate,
undo/redo, model-bound removal refusal, confirmed removal, save/reopen, and a
play-mode collision/interaction check. Optional
`ORR_ROOM_WALL_ITERATION_CAPTURE_DIR` stores a screenshot and acceptance
receipt in a new directory. To exercise the same-app Windows export path, also
provide an already-built trusted runtime with
`ORR_WINDOWS_EXPORT_RUNTIME` and a new destination with
`ORR_ROOM_WALL_ITERATION_EXPORT_BUNDLE`. These are integration-test hooks, not
runtime editor settings; the test does not claim an exit-key victory when the
authored wall intentionally blocks access to the key.
