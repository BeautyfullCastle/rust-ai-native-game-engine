# Room selected-wall two-corner resize verification — 2026-10-19

Base: `649bf2bfa74f67ae83abc3e32f634075c3beaf2a`, branch `su/room-wall-resize-20261019`. Verification ran locally on Linux against a frozen source cohort. No commit or remote publication was performed by the verification task.

## Results

**61 CPU tests passed**, with exact case-name inventories and no missing, repeated, failed or ignored tests:

- Selected resize: 8
- Existing selected movement: 9
- Checked Room updates: 4
- Height: 6
- Actor placement: 8
- New-wall footprint: 15
- Editor wall stamps: 3
- Pure wall stamps: 8

The first seven suites ran together as 53 tests, followed by the pure stamp cohort:

```sh
cargo test --locked -p orr_editor --features room-authoring --lib room_authoring -- --test-threads=1
cargo test --locked -p orr_editor --features room-authoring --lib room_wall_stamp::tests -- --test-threads=1
cargo clippy --locked -p orr_editor --features room-authoring --lib --test room_wall_resize_ui -- -D warnings
cargo check --locked -p orr_editor --lib
cargo check --locked -p orr_editor --lib --no-default-features --features room-authoring
```

Strict library/UI Clippy, default and minimal feature checks all passed.

Six actual production EditorApp software-Vulkan cases passed individually:

- New selected-wall resize: **1/1, 16.94 s**
- Existing selected-wall move: **1/1, 12.23 s**
- Existing wall height: **1/1, 15.54 s**
- Existing actor placement: **1/1, 13.57 s**
- New wall, grid Off: **1/1, 10.38 s**
- New wall, snapped grid: **1/1, 12.19 s**

Each case was explicitly executed with `--exact --ignored --test-threads=1`. All six passed their corresponding workflow’s exact individual-case/summary checks, typed receipt checks and screenshot PNG inventory checks. The exact-count validator’s nine self-tests passed. The new workflow YAML and embedded Python steps parsed successfully. Original mandatory workflows and suites remain unchanged.

To reproduce the new UI case, configure software Vulkan and set `ORR_ROOM_WALL_RESIZE_DIR` to a fresh, absent evidence directory, then run:

```sh
cargo test --locked -p orr_editor --features room-authoring --test room_wall_resize_ui selected_wall_two_corner_resize_preserves_identity_and_grows_collision -- --exact --ignored --test-threads=1
```

## Decisive resize oracle

The actual EditorApp fixture first authors a wall centered at X=1.85, Z=0.8 with half-width 0.15 and half-depth 0.15. A live player passes that original footprint. Two viewport corners near (1.7, -0.2) and (2.0, 1.8) retain the same intended center while growing half-depth to 1.0; ground projection introduces only the asserted sub-0.01 numerical tolerance. The player is then blocked along the same route. This distinguishes footprint growth from moving a distant wall into the route.

The acceptance asserts that first-corner and completed-draft states leave Frame checksum, undo history and saved scene unchanged, with original Body/Collider preserved. It checks staged numeric center/half extents, one Apply history entry, unchanged actor and GUID/name list, unchanged Y center and collider height, Undo/Redo, Save/fresh reopen and sidecar byte preservation. Six screenshots cover original route clearance, first corner, staged-only fields, applied geometry, reopen and collision.

Source review requested the same-center collision fixture before its first execution; no weaker collision run was used as evidence. The initial inventory exporter reused a workflow variable and mislabeled the CPU summary with the final GPU receipt. CPU case assertions had passed; the exporter was corrected to retain a separate CPU-count variable, then all inventory and receipt checks passed again without test changes.

## Source integrity and limits

All ten frozen source entries, covering the app route, authoring implementation, placement/footprint helpers, new tests, workflow and guide, were rechecked after all six GPU cases and remained unchanged. This verification document was edited afterward for publication. No test or compiler failure occurred.

Actual UI used llvmpipe (LLVM 19.1.7, 256 bits), synthetic egui input and owned fixtures. These results do not establish hardware-GPU, physical OS-input or Windows UI execution coverage. No user project was modified.
