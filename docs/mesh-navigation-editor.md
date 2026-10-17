# Scene-owned static-mesh navigation in the editor

## Integration status

The editor has an optional `mesh-navigation` feature for the distinct
`MeshNavigation` game. Run it with
```sh
cargo build --release -p orr_editor --features mesh-navigation
orr_editor --game mesh-navigation
```
The scene can start empty; the panel opens from the
`Static-mesh navigation` section of the editor.

The panel can create the built-in ramp/bridge draft, open a bounded canonical
`.orrmesh`, or import `.glb`, `.gltf`, or `.orrmodel` geometry. Imported source
geometry is cooked to a new `.orrmesh` output without mutating the host; output
paths are no-clobber. Set the 3D start/goal and agent settings, then use
`Place mesh agent` to submit the graph and settings through the actual local
host's document admission and undo history. The 3D viewport model is derived
from admitted `Frame` data, not from an unplaced draft. Play is blocked until a
valid mesh is placed, and again when edited draft settings no longer match the
scene pin; place the updated settings before starting Play. Local edits remain
available when unrelated host Frames refresh, but remain stale until explicitly
placed. Undoing the host pin hides its viewport model without discarding the
local draft. Scene changes clear the prior scene's mesh draft so it cannot be
placed accidentally into another scene.

The default command opens the shipped blank mesh scene. Use `--scene <path>`
for another scene. Expand the inspector section, enter the labeled Start/Goal
XYZ values and settings, and place the agent. The viewport shows the admitted
blue surface, orange route and pink point-agent marker. Save persists the
embedded graph and settings; Save As keeps unplaced drafts, while opening a
different document clears them. Reopened scenes do not need the source model
or `.orrmesh`. Use Play, Pause, Step and Stop on the normal editor toolbar.

## Verification

```powershell
$env:ORR_REQUIRE_GPU = '1'
cargo test --release --locked -p orr_editor --features mesh-navigation --lib --test deps --test mesh_navigation --test mesh_navigation_app -- --test-threads=1
```

The widget and production App tests exercise actual host admission/history,
Undo/Redo, Save/Save As, source-hidden reopen, Play/Pause/Step/Stop and GPU
readback. `ORR_MESH_EDITOR_CAPTURE_DIR` optionally retains the actual viewport
PNGs, scene and exact replay receipt. These tests use egui/WGPU Harness, not
manual native-window input. Exact-source Windows results, native captures,
publication and development integration are recorded separately in the PR;
the presence of test source alone does not prove successful execution.

## Ownership and authority

`orr_games::mesh_navigation_game::MeshNavigation` is a distinct compiled game,
not an alias for terrain navigation. `MeshScenePin` embeds at most 8,192 bytes
of canonical graph data, the graph revision and fixed-point agent settings in
the scene document. It has no external asset-root dependency on reopen or Save
As. Only a completely zero pin represents an empty authoring document.

The host's bake admission rejects authored entities, physics bodies/colliders,
noncanonical pins and authored runtime failure records. A valid pin creates
exactly one derived `MeshRuntime` agent. Successful runtime status is all zero;
step errors latch a bounded UTF-8 failure and do not silently move an invalid
agent. Scene files opened by the new host are bounded to 1 MiB before parsing.

The new panel keeps geometry and text-field drafts outside the simulation.
Creating or opening a mesh does not edit the document. Explicit placement
submits one `world.singleton.patch` through existing ERP document admission
and history. The skipped reserved field is never sent as reflected JSON.
The viewport model is reconstructed only from an admitted host Frame, not
from an unaccepted draft. Loaded `.orrmesh` files are bounded to 8,192 bytes;
the editor display requires exactly representable coordinates within +/-256.

`orr_navigation_runtime::mesh::decode_asset` and `decode_readback` share the
runtime's bounded canonical asset/path validation. The latter decodes supplied
state and bytes, not entity/list liveness. The remote view adapter checks live
agent identity/count and rejects missing, stale or oversized list contents.
Its `FrameView` argument must have the negotiated `MeshNavigation` registry,
like other typed bridge readers; it is not a generic arbitrary-schema decoder.

## Optional boundaries and limits

The feature is opt-in and does not replace terrain navigation. It uses the
separate static-mesh graph, embedded scene pin, and existing model presentation
path; existing terrain navigation keeps its own game, caches and admission
semantics.

This remains a point-agent/open-sky triangle graph: no radius/headroom/step,
physics controller, collision sweep, independent collider-world proof,
crowd avoidance or clearance-aware navmesh is implied. The scene defines its
world by the embedded walkable graph, so its world revision equals that graph
revision. Per-tick distance is an absolute route-parameter increment, not a
physical speed guarantee. See [graph](static-mesh-navigation.md) and
[Frame runtime](static-mesh-runtime.md) for their precise contracts.

No Windows game export or exported-game execution is added here. Complete game
creation/save/play/export, #94 completion and engine completion remain
unverified; #2's original 22 follow-up issues are a separate denominator.
