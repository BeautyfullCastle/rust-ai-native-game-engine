# Collider selection outline verification — 2026-10-19

Base: `38dd4f3fd75a9ea4fd2a0892d58854d911b8939e`. Local verification ran on Linux on 2026-10-18 UTC. Independent source review approved the bounded shape geometry, transform handling, atomic finite append, horizontal-plane behavior and caller integration. Existing viewport3d/renderer files and picking behavior were left unchanged.

## Gates and evidence

- **30 CPU tests passed:** seven new outline tests, six framing tests and seventeen existing camera tests. Used `cargo test -p orr_editor --lib --features room-authoring` with `selection_outline`, `frame_selection` and `room_camera` filters, serially. Logs: `/tmp/selection-outline-v1-{cpu,frame,camera}.log`.
- Default library, minimal models and minimal room-project checks passed: `/tmp/selection-outline-v1-{default,models,room}.log`.
- Strict production library Clippy passed separately. Final library plus integration-test Clippy passed with `--features room-authoring -- -D warnings`; final integration target rebuilt successfully. Logs: `/tmp/selection-outline-production-clippy.log` and `/tmp/selection-outline-v3-{ui-build,clippy}.log`.
- **Room UI: 1/1 passed, 6.29 s, three PNGs.** `/workspace/shared/selection-outline-room-ui-20261019-v2`.
- **Yard3D UI: 1/1 passed, 2.08 s, two PNGs.** `/workspace/shared/selection-outline-yard-ui-20261019-v2`.

Both directories include `acceptance.json`, `run.log`, source manifest/stability proof and screenshots. The Room case retains the original framing button/F and guard/invariant regression, and now checks the selected GUID and thin-box outline against twelve projected true collider edges. Its receipt records 2,762 outline-colored pixels. The Yard case selects known fixture GUIDs through the hierarchy, asserts their expected shapes, frames a rotated thin box and capsule, and compares outline pixels to projected analytic geometry; counts were 3,261 and 3,036. Authoring Frame/history/selection and scene files remained unchanged.

The helper replaces the prior oversized selection cube at the app caller with actual collider-shape wire geometry. Imported model bounds are not used. CPU tests cover thin and rotated boxes, sphere circles, capsule rings/sides/caps, plane parity, source preservation and invalid-geometry atomic refusal.

## Preserved failures

Initial strict integration Clippy rejected constant-size `chunks_exact(4)` in the test. The test uses `as_chunks::<4>().0.iter()` instead; no lint allowance was added. Original log: `/tmp/selection-outline-v1-clippy.log`.

Both first actual UI runs failed the test color classifier. Screenshot color histograms showed the encoded wire color was sRGB `[255, 237, 124]`: the original blue-channel ceiling excluded it, while Yard orange fill could satisfy the broad classifier. The evidence-based test-only correction matches that encoded wire color within two units per RGB channel. Spatial distance, margin and minimum visible-pixel assertions were retained, and Yard gained exact shape-per-GUID assertions. No production geometry was changed to obtain the passes. Original logs, images and source proofs remain in `/workspace/shared/selection-outline-room-ui-20261019-v1` and `/workspace/shared/selection-outline-yard-ui-20261019-v1`.

## Frozen source

Final manifest `/tmp/selection-outline-v3-source.sha256` SHA256: `77ce7a2e92ae2ad73bb307953f22df15404db4c8bc3b5fb23ef6c78b5ef23d0d`. All five entries were unchanged through both final UI executions and checked again afterward:

```text
83b776ef020c2db6538070d8e940d5d718bde3c2d32e9df3e85a33680c3e5833  crates/orr_editor/src/app.rs
ee085e8e4a9117c9ad82dd1b3fbe122a8b5925871ece4d5d33a7011cd5d57e70  crates/orr_editor/src/app/selection_outline.rs
76401a340a3a3138941c487970ee63f99a7029388cb595601d7431c4bda9908d  crates/orr_editor/src/app/selection_outline/tests.rs
328523878143e6868a914881285169888e63af19f94aa00081510438bdeaec61  crates/orr_editor/tests/frame_selection_ui.rs
9a19a1c814cf91432194e224d83a6148726c7aa3354f7d7fc5a4152007ecc1fb  docs/editor-frame-selected-collider.md
```

## Limits

Actual production EditorApp rendering used llvmpipe (LLVM 19.1.7, 256 bits), synthetic input and owned Room/Yard fixtures. Geometry/pixel assertions apply to these tested views and shapes. Hardware GPU, physical OS input and Windows execution were not established by these local runs.
