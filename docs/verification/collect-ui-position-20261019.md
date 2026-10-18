# Collect UI position editing verification — 2026-10-19

## Results

**68 CPU tests passed:** 19 position-state tests, 18 preview-state tests, two actual EditorApp control tests, six existing authoring-panel tests, 12 production Collect UI tests and 11 existing runtime-policy tests. The unchanged production sample regressions used their retained previously built executable. Two GPU tests were ignored in the CPU run and each executed explicitly afterward: **both passed**.

Verified commands and selectors:

- `cargo test -p orr_editor --features collect-ui --lib collect_ui_ -- --test-threads=1`
- Production sample selectors `collect_ui::tests::` and `collect_app::tests::`, with `collect-ui` enabled
- `cargo check -p orr_editor --lib`
- `cargo check -p orr_editor --no-default-features --features collect-ui --lib`
- `cargo clippy -p orr_editor --features collect-ui --lib --tests -- -D warnings`
- Exact ignored GPU cases under `app::collect_ui_preview::tests::`: `actual_editor_position_gpu_stages_applies_and_cancels_offsets` and `actual_editor_preview_gpu_matches_production_and_never_dispatches`

Default/minimal feature checks and strict Clippy passed. Existing authoring-panel implementation and production runtime sources remain unchanged.

## Actual editor acceptance

The new position workflow passed with 13 retained images and six exact production-UI pixel-parity cases. The existing preview regression also passed, retaining 14 images and seven parity cases. Source identities remained unchanged through both final runs; full evidence is retained separately.

The production EditorApp test verifies a drag ghost without rebuilding the preview context, one candidate build on release, staging that changes pixels without changing accepted authoring state, exact pixel restoration on Cancel, and explicit Apply producing one undoable edit. It checks old/new action hit locations, Undo, Save/reopen, portrait display scaling, and moving a parent without rewriting descendant offsets.

Cancellation is exercised for Escape, focus loss, screen/size changes, close, stale accepted documents, same-path scene replacement, Play, pointer leave and late release. Host state stays unchanged during authoring. Full editor captures retain visible controls alongside the rendered preview.

## Failures and fixes retained

- Initial compilation found six test calls using `is_disabled` directly on a test node. Importing its trait alone was insufficient, and a second compiler failure was preserved. The test now queries the underlying accessibility node with `accesskit_node().is_disabled()`.
- The first GPU run exposed a cancellation failure. A diagnostic-only rerun identified the explicit focus-loss event: the harness event did not also change its raw focused flag. The production interruption guard now honors both the focused state and an explicit focus-loss event. The cancellation assertions were retained, and the final GPU run passed them.

Final CPU, feature-boundary and strict checks were repeated after the production focus guard correction. Original failed runs are retained; no assertions were disabled or skipped to obtain acceptance.

## Limits

GPU acceptance uses software Vulkan and synthetic editor events. It proves rendering parity and authoring/host isolation, not native OS input, physical-device behavior, hardware-GPU performance or Windows execution. Preview action observations do not dispatch gameplay commands. Position editing changes only staged offsets until explicit Apply through the existing panel action.
