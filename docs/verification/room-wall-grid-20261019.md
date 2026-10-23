# Optional Room wall grid verification — 2026-10-19

Local Linux verification against base `0db3a1e06f79f6c786b228eeb61e1c3ed26181bd`.
Optional wall-footprint snapping defaults to Off. Available steps are 0.25, 0.5,
1 and 2 units. Each ground corner is snapped before exact center/half-extent
calculation. Signed half-step ties round away from zero, including negative
coordinates. Checked i128 intermediate arithmetic prevents overflow; invalid or
unrepresentable results are refused without clamping or saturation. Off retains
the original unsnapped path. Changing grid mid-gesture cancels the gesture without
staging or mutating the scene. Independent source and workflow reviews approved
the changes; app/renderer APIs remain unchanged.

## Passing checks

- **26 distinct CPU tests passed**: 15 footprint tests (six added grid cases and
  nine prior cases), three existing editor stamp regressions, and eight pure
  stamp tests. Commands used `cargo test -p orr_editor --lib --features
  room-authoring` with selectors `room_authoring` and `room_wall_stamp::`, serially.
  Logs: `/tmp/room-wall-grid-v1-cpu.log` (18 passes) and
  `/tmp/room-wall-grid-v1-stamp.log` (8 passes).
- Default library and minimal `--no-default-features --features room-authoring`
  checks passed: `/tmp/room-wall-grid-v1-default.log` and
  `/tmp/room-wall-grid-v1-minimal.log`.
- Strict library/UI Clippy passed with `--lib --test room_wall_footprint_ui
  --features room-authoring -- -D warnings`:
  `/tmp/room-wall-grid-v1-clippy.log`.
- UI integration build passed: `/tmp/room-wall-grid-v1-ui-build.log`.
- **Snapped actual UI: 1 passed, 0 failed**, 12.90 seconds.
  Exact test: `snapped_wall_cancels_grid_changes_and_preserves_gameplay`.
- **Original Off actual UI: 1 passed, 0 failed**, 10.39 seconds.
  Exact test: `two_corner_wall_stages_creates_persists_and_blocks_player`.

Both UI runs used the production EditorApp and passed on the first frozen run.
No failed compilation or test run occurred in this grid cohort. Earlier footprint
failures remain documented separately in `room-wall-footprint-20261019.md`.

## Separate UI evidence

Snapped evidence: `/workspace/shared/room-wall-grid-ui-20261019-v1/`, containing
`acceptance.json`, `run.log`, source-before/stability records and six PNGs. It adds
`00-grid-change-canceled.png` to the five lifecycle captures. The receipt confirms
exact snapped field values, grid-change cancellation without mutation, draft-only
staging, Escape preserving selection, Create, Undo/Redo/Save/Reopen, player
collision and preservation of actors/models/camera.

Off evidence: `/workspace/shared/room-wall-grid-off-ui-20261019-v1/`, with its own
`acceptance.json`, `run.log`, source-before/stability records and five PNGs.
It independently repeats the original unsnapped staging/Create/persistence/
collision flow. Its receipt intentionally marks snapping-specific fields false;
those checks belong to the separate snapped case and are not failures.

CPU tests separately verify all grid steps, signed ties and neighboring values,
extreme fixed-point values, exact center/extent results in either corner order,
Off identity, stale-grid cancellation, and preservation of name/host/files when
staging or rejecting an invalid second corner.

## Frozen source evidence

All five entries remained unchanged across both UI runs and subsequent checksum
verification. Manifest `/tmp/room-wall-grid-v1-source.sha256` has SHA-256
`df0e925e1974928512fb5988cd8a9ffab9dab14bd3bc397a8cd1f77f1a7573a9`.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/room_authoring_panel.rs` | `9f37010306d62f6034e700dd64894df6bf0ffd6c9e32098cd4ee175cac2f4d7a` |
| `crates/orr_editor/src/room_authoring_panel/wall_footprint.rs` | `285b307144cda59d59024b74e29d405ade0d142b7ba4745f63ff309f0f271701` |
| `crates/orr_editor/src/room_authoring_panel/wall_footprint/tests.rs` | `54cfdb0c49955be3941ac90356a4fca5ed2f08e019172001f114408f35b5e4c0` |
| `crates/orr_editor/tests/room_wall_footprint_ui.rs` | `541680b345e4dd3c4b56ba5c18ca6846fd4f142bed71db102d639379778ce975` |
| `docs/editor-room-wall-footprint.md` | `2f7dfcd4de0ceb3283d00eea7fde723dcc5d21c4c21956d968b96f4067795480` |

## Limits

UI rendering used llvmpipe (LLVM 19.1.7, 256 bits) software Vulkan with synthetic
egui/AccessKit input. Physical GPU, physical OS input, native desktop-window and
Windows execution were not verified here. Remote CI/publication are separate
gates. This report was added after source freezing; `git diff --check` is clean.
