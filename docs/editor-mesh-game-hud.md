# Mesh game HUD

The Mesh game HUD is a presentation sidecar for a saved Mesh Navigation scene. It displays a title and one message selected from the current agent state: moving with positive speed, arrived, or idle. An optional tick readout is presentation-only; it does not change the scene, navigation graph, or simulation.

The HUD is opt-in by sidecar presence. Without a matching `<scene filename>.mesh-hud.json`, no HUD is drawn. The sidecar is bound to the exact scene filename and admitted graph revision, and changes are applied through the Mesh game HUD panel's local Undo/Redo history and saved explicitly. Removing the saved HUD requires the panel's confirmation control. Invalid or stale sidecars fail closed instead of silently changing the scene.

## Use

1. Create or open a local Mesh Navigation scene and save it.
2. Open **Mesh game HUD** and set the title, idle/moving/arrived messages, font size, and optional tick display.
3. Select **Show mesh game HUD**, then choose **Apply game HUD**. Apply is an in-memory presentation edit; save it explicitly with **Save game HUD**.
4. Use Undo/Redo to review the local HUD edit. Play remains blocked while the presentation edit is unsaved.
5. Export through **Export mesh game**. A HUD-enabled bundle contains the scene, its `.mesh-hud.json` sidecar, `orr.mesh-bundle.json`, and `orr_mesh_player.exe`. The standalone player must include the dedicated HUD capability marker when the sidecar is present.

The HUD reflects the navigation agent's current status; it is not a mission system, objective tracker, dialogue system, or proof that a game is complete. It does not add gameplay state or physical input.

## Windows GPU acceptance

The actual Editor UI acceptance uses AccessKit widget actions and a hardware Vulkan adapter. It captures the full composited App frame, including the HUD overlay, rather than relying only on the 3D viewport texture. It compares a no-HUD frame with a matching paused moving frame, verifies the arrival-state render, confirms the scene checksum stays unchanged, and checks the exported four-file bundle.

From the repository root, set fresh absolute output locations and the matching player executable, then run:

```powershell
$env:ORR_MESH_GAME_HUD_ACCEPTANCE = '1'
$env:ORR_REQUIRE_GPU = '1'
$env:WGPU_BACKEND = 'vulkan'
$env:ORR_MESH_GAME_HUD_CAPTURE_DIR = 'E:\Evidence\mesh-hud-captures'
$env:ORR_MESH_GAME_HUD_OUTPUT = 'E:\Evidence\mesh-hud-captures\mesh-game-hud.scene.yaml'
$env:ORR_MESH_GAME_HUD_EXPORT_OUTPUT = 'E:\Evidence\mesh-hud-export'
$env:ORR_MESH_EXPORT_RUNTIME = 'E:\Evidence\orr_mesh_player.exe'
cargo test --release -p orr_editor --features mesh-navigation,mesh-export --test mesh_game_hud_ui -- --ignored --exact mesh_game_hud_applies_renders_saves_reopens_and_exports --test-threads=1 --nocapture
```

This acceptance does not claim native standalone execution or exhaustive verification of every possible HUD string, display size, or navigation route.
