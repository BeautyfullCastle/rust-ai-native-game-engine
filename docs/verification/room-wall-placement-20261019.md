# Room wall viewport placement verification — 2026-10-19

Base: `7a0aed418515f7bd508116e9c1586feef99130ef`, branch `su/room-wall-placement-20261019`. Local Linux verification ran on 2026-10-18 UTC. Independent source review approved the checked transaction, exact original-row guard, staged context and app input ownership.

## Results

- **53 CPU tests passed:** four checked-update tests, nine placement-panel tests and forty existing height/actor/footprint/editor-stamp/pure-stamp regressions. Commands used `cargo test -p orr_editor --lib --features room-authoring` with `room_authoring` (45) and `room_wall_stamp::tests` (8), each serially.
- Default library and minimal `--no-default-features --features room-authoring` checks passed. Strict library plus `room_wall_placement_ui` Clippy passed with `--features room-authoring -- -D warnings`.
- New actual production EditorApp case **passed 1/1 in 11.99 s**: `/workspace/shared/room-wall-placement-ui-20261019-v2`. Receipt confirms pre-Apply Frame/history/files unchanged, Escape cancellation, preserved identity/Y/dimensions, Undo/Redo/Save/reopen and Play collision.
- Existing UI targets were rebuilt against the same final production source, then passed separately:
  - Height: **1/1, 15.69 s**, `/workspace/shared/placement-height-ui-20261019-v1`.
  - Actor: **1/1, 14.02 s**, `/workspace/shared/placement-actor-ui-20261019-v1`.
  - Footprint Off: **1/1, 10.73 s**, `/workspace/shared/placement-footprint-ui-20261019-v1`.
  - Grid footprint: **1/1, 12.36 s**, `/workspace/shared/placement-grid-ui-20261019-v1`.

Evidence directories contain receipts/screenshots. Compiler logs are `/tmp/wall-placement-v2-{cpu,stamp,ui-build,default,minimal,clippy,regression-build}.log`; actual logs are `/tmp/room-wall-placement-v2-ui.log` and `/tmp/placement-{height,actor,footprint,grid}-v1-ui.log`.

## Fixes and original failures

The initial CPU cohort recorded **37 passed, seven failed** because the new panel fixture read walls without selecting one. The fixture now explicitly selects a wall, matching the action prerequisite. Original `/tmp/wall-placement-v1-cpu.log` and source manifest remain retained.

Review added refusal to arm when unrelated name, dimension or Y drafts differ from the original wall, while retaining semantically equivalent numeric text. Staged placement preserves the original name byte-for-byte. The checked update compares fresh checksum and original wall row inside the authoritative transaction, including name-only changes that a physics checksum alone may not identify. The legacy crate-private update API is retained with a documented function-local dead-code allowance; no broad lint suppression was introduced.

The first actual UI attempt stopped before app creation because the software Vulkan environment was not sourced. `/tmp/room-wall-placement-v1-ui.log` is retained. Sourcing the established `/workspace/shared/software-vulkan/env.sh` enabled the unchanged-source pass. After all four regression processes returned success, a convenience `tail` summary command failed; the terminal logs were read separately and all four successes confirmed. That summary-command failure was not a test failure and prompted no test rerun.

## Frozen source

Manifest `/tmp/wall-placement-v2-source.sha256` SHA256: `23c9bc91dceb63f32577aef44014ce307da286e2d54bb8403064517ac95e7c7d`. All ten entries stayed unchanged through all five UI cases and were rechecked afterward:

```text
18b9bdb91a045badfbfa86dd965effb77e614b403efcb38193e09284ab8dc84c  crates/orr_editor/src/app.rs
a09ca845505eb4df42fd181882d3c2fad3b820febc7ca1b7d5cb009450b482e5  crates/orr_editor/src/app/frame_selection.rs
b3e760addb7e677b1c6413ce113cf4f8036ab639fb3d10980bec16f196bfcf72  crates/orr_editor/src/app/room_wall_placement.rs
88a83ad12247c8c92a008130666e7ecb8a191ec21f9b7de1830969a0840f048e  crates/orr_editor/src/room_authoring.rs
0731f3c0f8fb56f9757ed12d2b71ff7da90dbcfab2ffaaf8d86fa10f6990638f  crates/orr_editor/src/room_authoring_panel.rs
805bb174b064953800db51b42f29d31d4200d7bdedf4a8e139120d7300b40bb7  crates/orr_editor/src/room_authoring_panel/wall_placement.rs
9083b6b60d9a2a5fe059cce317a43ab230b8b341c6ebc523161b0cfb6a8521f4  crates/orr_editor/src/room_authoring_panel/wall_placement/tests.rs
af529ef154ab4a54447e1dd136493bfc077202619c26fe74bef67bab2865b137  crates/orr_editor/src/room_wall_update_checked_tests.rs
037a6367c7b2c5160d5785fda444686276ee0642545aa425a0e330c72c5ee5c9  crates/orr_editor/tests/room_wall_placement_ui.rs
10599e2f8c30c0dd8651bd90784d58ba7e611f75a8b4bc33c9861c89baa0d614  docs/editor-room-wall-placement.md
```

After the final source-cohort recheck, the guide alone was clarified to state the already-tested prerequisite: apply or reread unrelated name/height/size drafts before arming. Production and test files remained unchanged.

Actual UI used llvmpipe (LLVM 19.1.7, 256 bits), synthetic input and owned fixtures. These local runs do not establish hardware-GPU, physical OS-input or Windows execution coverage. No user project was modified.
