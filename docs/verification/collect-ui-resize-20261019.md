# Collect UI resize editing verification — 2026-10-19

## Results

**88 CPU tests passed:** 19 resize-state tests, 19 position-state tests, 18 preview-state tests, three actual EditorApp control tests, six authoring-panel tests, 12 production Collect UI tests and 11 runtime-policy tests. The unchanged sample regressions reused their retained executable with source and binary identities recorded.

Default-feature and minimal `collect-ui` library checks passed, as did strict Clippy including tests. All three ignored GPU cases were then executed explicitly: **resize, position and preview each passed with zero ignored tests**. Final source snapshots stayed unchanged through verification.

The resize and position runs each retained 13 images and six exact production-renderer pixel-parity cases; preview retained 14 images and seven parity cases. The CI receipt predicates and every expected PNG dimension were validated against these outputs.

Verified commands and selectors:

- `cargo test --locked -p orr_editor --features collect-ui --lib collect_ui_ -- --format=pretty --color=never --test-threads=1`
- Production sample selectors `collect_ui::tests::` and `collect_app::tests::`, with `collect-ui` enabled
- `cargo check --locked -p orr_editor --lib`
- `cargo check --locked -p orr_editor --no-default-features --features collect-ui --lib`
- `cargo clippy --locked -p orr_editor --features collect-ui --lib --tests -- -D warnings`
- Exact GPU names under `app::collect_ui_preview::tests::`: `actual_editor_resize_gpu_stages_applies_and_cancels_sizes`, `actual_editor_position_gpu_stages_applies_and_cancels_offsets`, and `actual_editor_preview_gpu_matches_production_and_never_dispatches`

## Verification scope

The resize gate requires 19 exact state tests and one exact actual EditorApp control test. Existing position, preview, authoring-panel and production runtime suites remain regression requirements. Software-Vulkan acceptance runs the ignored resize test explicitly with `--ignored --exact` and requires one passing test with zero ignored cases; an ignored CPU listing is not GPU acceptance.

The new CI workflow runs on Linux and Windows, checks default and minimal feature boundaries, and runs strict Clippy including tests. Linux additionally installs and explicitly selects software Vulkan, validates the acceptance receipt, and checks all 13 expected PNG dimensions. Its resize test uses the actual EditorApp and production Collect renderer.

## Actual editor acceptance coverage

Resize changes only a selected node's staged size. Pointer motion draws a ghost without rebuilding the preview context; release builds one validated candidate. Apply uses one undoable authoring edit. Cancel restores accepted preview pixels. The test covers expanded button hits, parent clipping with child anchors at 0, 500 and 1000, portrait display scaling, Undo and Save/reopen.

Interruption checks cover Escape, focus events and focused snapshots, screen/viewport changes, close, stale documents, same-path scene replacement, Play, pointer leave and late release. Mode and selection changes cancel staged work; an inactive Move selection cannot cancel the active Resize draft. Host state and project manifests remain isolated from authoring.

## Failures retained

The first default-feature check found the new app module missing its `collect-ui` feature guard. The guard was corrected, and the original compiler failure remains retained. An earlier successful CPU run was superseded because source changes overlapped compilation; final acceptance must use a stable source snapshot.

## Limits

GPU acceptance uses software Vulkan and synthetic editor events. It does not establish native OS input, hardware-GPU performance or Windows GPU behavior. Preview actions are observed without dispatching gameplay commands. Full logs, manifests and captures are retained separately.
