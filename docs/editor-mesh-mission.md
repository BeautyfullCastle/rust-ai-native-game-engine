# Mesh mission authoring

The optional `mesh-mission` feature adds scene-embedded collectibles and an exit to Mesh Navigation. The mission is ordinary reflected gameplay state in the saved scene, not a sidecar. It is tied to the admitted mesh graph revision; invalid or mismatched configurations fail closed.

## Author a mission

1. Create or open a local Mesh Navigation scene, create its walkable mesh, place the point agent, and save the scene.
2. Open **Mesh mission**, enable the mission, choose one to eight collectible positions and radii, and set the exit position and radius. Every active point must lie exactly on the admitted graph projection; values use fixed-point scene decimals.
   Use **Pick collectible N in viewport** or **Pick mission exit in viewport** to select the nearest visible admitted walkable surface instead of typing XYZ. The click stages exact fixed-point coordinates and preserves the radius; it does not edit the scene or history. Invalid surface clicks stay armed; **Cancel mission point picking**, Escape, focus loss, or changed scene/graph context cancel without applying. Finish or cancel picking before transport, scene/workspace changes, or export.
3. Select **Apply mesh mission** to validate the complete candidate and commit it as one normal ERP scene edit. This does not use a separate editor-only history. Use the editor's global Undo/Redo, and save the scene explicitly.
4. Play the scene. The panel reports the collected count and only reports **Mission complete** after all collectibles and the exit are reached. Route arrival by itself is not a win.
5. **Remove mesh mission** requires explicit confirmation and writes the canonical all-zero disabled singleton. Save the scene to persist the removal.

Draft fields are local and block Play until applied or reloaded. Apply, Undo/Redo, and simulation use the normal host/frame path. Mission markers and status are presentation of the authoritative mission config and run singleton; they do not add collision, physics, combat, dialogue, or persistent profile storage.

## Export

The game and host feature profiles must agree. A transitive dependency enabling mission gameplay without the host's `mesh-mission` feature is rejected during scene admission rather than mislabelling the augmented registry as a legacy frame stream. Enable the editor-level feature to propagate the coherent profile through game, host, and player.

The mission remains in the scene file and is included in the ordinary three-file Mesh game export; no mission sidecar is created. Every build with `mesh-mission` enabled exports the top-level game profile `mesh-mission-v1` and requires the dedicated `.orrmis` PE section beginning with `ORR_MESH_MISSION_V1`, even when the scene has no enabled mission, because the feature adds mission singletons to the runtime registry. A mission-feature player rejects legacy bundles without that profile, and a feature-off player rejects bundles carrying it. Feature-off exports omit the profile and retain their legacy manifest format. The player re-admits the saved scene and any enabled mission against its embedded graph before running; no external package or mission-sidecar source is required.

## Windows GPU acceptance

The ignored actual-UI acceptance drives the production Editor with AccessKit actions and requires a hardware Vulkan adapter. It sets the agent's distance per tick through the production mesh-navigation field, compares viewport-only GPU pixels for the authored world markers before and after applying the mission and after reopening, and separately captures full composited App frames for the mission overlay. Bounded simulation steps must collect both objectives, set the authoritative win flag, and reach `Arrived`; the game overlay must change from in-progress to complete. While paused, the acceptance seeks to tick zero and verifies progress resets, then seeks back to the recorded winning tick and verifies both progress and the full frame checksum replay exactly. Stop must restore the authored frame, followed by a same-App export. `physical_os_input` is false: this is production-widget/UI automation, not an operating-system mouse/keyboard test. It does not claim physics-based character control, collision, networking, or a general-purpose quest system.

From the repository root, set fresh absolute output locations and the matching Mesh player executable:

```powershell
$env:ORR_MESH_MISSION_ACCEPTANCE = '1'
$env:ORR_REQUIRE_GPU = '1'
$env:WGPU_BACKEND = 'vulkan'
$env:ORR_MESH_MISSION_CAPTURE_DIR = 'E:\Evidence\mesh-mission-captures'
$env:ORR_MESH_MISSION_OUTPUT = 'E:\Evidence\mesh-mission-captures\mesh-mission.scene.yaml'
$env:ORR_MESH_MISSION_EXPORT_OUTPUT = 'E:\Evidence\mesh-mission-export'
$env:ORR_MESH_EXPORT_RUNTIME = 'E:\Evidence\orr_mesh_player.exe'
cargo test --release -p orr_editor --features mesh-mission,mesh-export --test mesh_mission_ui -- --ignored --exact mesh_mission_collects_then_wins_after_reopen_and_exports --test-threads=1 --nocapture
```
