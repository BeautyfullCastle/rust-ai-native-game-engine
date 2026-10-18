# Mesh Play goal input

Mesh Play goal input lets a local player retarget the admitted mesh agent by clicking a walkable surface while realtime MeshNavigation Play is running. It uses the existing managed game-input lease and the ordinary simulation input path; it does not issue editor patches or create edit-history entries.

## Use

1. Open or create a MeshNavigation scene and place a valid mesh agent.
2. Start realtime Play, then enable **Mesh Play input**.
3. Click an in-bounds walkable surface in the game viewport. The click is projected against the current admitted mesh graph and queued for the next simulation tick.
4. Disable input when finished, or stop Play. The managed input release is handled by the editor's input state machine.

Only live local Play at the current head accepts clicks. Paused/replay sessions, stale frames or graph revisions, UI focus/dialog input, non-finite/out-of-range coordinates, and misses are rejected without changing the goal. A valid click queues the existing one-shot mesh-goal input; the MeshStep runtime replans through the normal managed-input path. The graph bounds and managed raw-coordinate encoding are checked before enqueueing.

This is point-navigation input, not a physics controller: it changes the mesh agent's route, not collision, acceleration, or arbitrary scene state. It does not edit the scene document and therefore does not add an Undo entry. Stop returns the scene to its saved/edit state.

## Acceptance

The opt-in hardware UI acceptance uses `ORR_MESH_PLAY_INPUT_ACCEPTANCE=1`, `ORR_REQUIRE_GPU=1`, `WGPU_BACKEND=vulkan`, and a fresh absolute `ORR_MESH_PLAY_INPUT_CAPTURE_DIR`. It exercises the real Editor UI and managed local host with synthetic pointer input; it does not claim physical OS input or a full physics controller.

The same acceptance flow exports through the editor using `ORR_MESH_PLAY_INPUT_RUNTIME` and a fresh sibling `ORR_MESH_PLAY_INPUT_EXPORT_DIR`. Interactive bundles declare the strict `mesh-click-to-move-v1` input profile and require a player executable advertising `ORR_MESH_INPUT_V1` in its dedicated PE section. Legacy bundles without that profile remain replay-only in the standalone player. The profile adds no scene fields or sidecar files.

MeshNavigation build identity advances to `0x4d45534800000002` for the new input semantics. The YardInput byte layout and scene component schema are unchanged. A goal remains held until a later authoritative frame confirms consumption; only then does the editor send neutral input. This avoids losing a click between an ERP acknowledgement and the next simulation tick.
