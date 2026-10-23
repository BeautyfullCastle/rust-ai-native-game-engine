# Portable mesh lighting presets verification — 2026-10-19

Base: `770acd9460d9ad34ab31ffe51abfbcfe95e03488`. Local verification ran on Linux on 2026-10-18 UTC. Independent source review approved the scoped implementation.

## Results

- **44 CPU tests passed:** twenty-three new schema/store tests, five new panel tests, and sixteen existing lighting document/panel/store regressions. Command: `cargo test -p orr_editor --lib --features mesh-mission mesh_lighting -- --test-threads=1`.
- Default library and minimal `--no-default-features --features mesh-navigation` checks passed. Strict library plus `mesh_lighting_preset_ui` Clippy passed with `--features mesh-mission -- -D warnings`.
- Actual production EditorApp cross-scene UI **passed 1/1 in 24.64 s**, with nine PNGs: `/workspace/shared/lighting-preset-ui-20261019-v2`.

Receipt confirms distinct source/target graphs, portable preset export, no-overwrite save, corrupt-load draft/history preservation and unchanged Frame/history/files/pixels during staging. Apply changed 24,312 pixels. Undo restored the exact pre-Apply viewport hash; Redo and fresh target reopen matched the exact applied hash. Target shadow bounds `[3, 1.25, 2, 18]` were preserved. The portable preset was 420 bytes, SHA256 `ffa09e1a7c4ab99702b4252bb15371e3403123e0e8bbb01ba3572eb08c45706d`.

The evidence directory contains `receipt.json`, nine screenshots, `run.log`, `source-before.sha256` and `source-stability.log`. Final logs: `/tmp/lighting-presets-v2-{cpu,ui-build,default,minimal,clippy,stability}.log`, plus `/tmp/lighting-presets-v2-ui.log`; final layout-test rebuild/strict logs `/tmp/lighting-presets-v3-{ui-build,clippy,stability}.log`.

## Provisional evidence and reviewed fixture corrections

The first cohort passed its tests/build/checks/Clippy, but its final hash check detected a concurrent test-only edit to `store_tests.rs`. That cohort is provisional, not the final frozen proof; its logs and v1 source manifest are retained. A complete final CPU/build/check/Clippy chain was rerun after all writes stopped, followed by the actual UI and successful ten-file hash check.

The reviewed test-only corrections preserve original Windows absolute temp paths, capture the FIFO subprocess stdout/stderr so its child summary cannot contaminate exact test counts, and change replacement-fixture length to avoid relying on coarse modification timestamps. Final CPU output has one 44-test summary; the FIFO child is part of its parent test, not an additional distinct test. No admission policy was relaxed. The existing protected `mesh_workspace/tests.rs` unused `checked_scene_path` import warning remains visible; it was not edited or suppressed. No actual UI or final test failed.

## Frozen source

Final manifest `/tmp/lighting-presets-v3-source.sha256` SHA256: `bbf9a9ea1fa213ebcd8ce664e880a080a80cef2d236c2eafe4ab2986c60b949a`. All ten entries stayed unchanged through final verification:

```text
0b1af2eb37ed9aba5e3aba138621a1f8b5da21c01a46e94594378c1e0810be57  crates/orr_editor/src/lib.rs
10970601f19a2d88137ec0010204a9d24efc4db8f71439933afcacb4add5ace5  crates/orr_editor/src/mesh_lighting_panel.rs
c27d36d116aed4f8a3103a9768ddeaa8631e18b12f397a0a60368c39d5b2eb64  crates/orr_editor/src/mesh_lighting_panel/presets.rs
f89987f2f7be7184af8cee916b68c7bf2cedb0fb0e09c09782e1143fe425fd01  crates/orr_editor/src/mesh_lighting_panel/presets_tests.rs
504153268a74fa0d272eb6ab4fb6280af8cbbcae5edc5f3d8f568b7d51bb0c61  crates/orr_editor/src/mesh_lighting_preset.rs
62fee1ef283cb6a0a1739df8fcc84a70933fc14a5a189cd455039c00964fd081  crates/orr_editor/src/mesh_lighting_preset/store.rs
bd2daeedc1ae6ae6d36f1e28e298d6466be38a5df5f952635980680c393e8b7d  crates/orr_editor/src/mesh_lighting_preset/store_tests.rs
5ef1e2548022d56664965fdd9906335417014a6c9c670e2da8ae5200993bf436  crates/orr_editor/src/mesh_lighting_preset/tests.rs
329e2d451f14e1a0140ef4af8e6fc4f7420d9a9d90b022502efcea617e6fcf3f  crates/orr_editor/tests/mesh_lighting_preset_ui.rs
d563d29c8b4557ad65c13546633d5ec3c47b54109831f16f0f6398ca24f43081  docs/editor-mesh-lighting-presets.md
```

## Limits

The actual case used llvmpipe (LLVM 19.1.7, 256 bits), software Vulkan and synthetic egui input in owned scene fixtures. This establishes no physical OS input, hardware-GPU, Windows execution, runtime or export acceptance. Windows test counts inferred from cfg source are not executed results. Renderer code, scene schema and dependencies were unchanged.

## Full Inspector layout follow-up

The original eight-viewport-image UI run passed in 22.69 s and remains preserved in `/workspace/shared/lighting-preset-ui-20261019-v1`. Before publication, a test-only extension added a full 1400×1050 editor capture and asserted that the path input, Save/Load controls and staged-status rectangle all remain inside the fixed 340-pixel Inspector and screen. The final nine-image run passed those assertions (`preset_controls_layout.controls_visible_and_inside_inspector = true`). Screenshot `09-preset-controls-layout.png` supplies the additional layout evidence. Production sources were unchanged; the updated test and guide were included in the final v3 manifest.
