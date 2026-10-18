# Collect screen preview verification — 2026-10-19

## Results

**46 CPU tests passed:** 16 preview-state tests, one actual EditorApp control/isolation test, six existing authoring-panel tests, 12 production Collect UI tests and 11 existing Collect runtime-policy tests. The production sample regressions used the retained executable from the preceding unchanged-runtime build. The ignored GPU acceptance was executed separately and passed **1/1**.

Commands and selectors:

- `cargo test -p orr_editor --features collect-ui --lib collect_ui_preview`
- `cargo test -p orr_editor --features collect-ui --lib collect_ui_panel::tests::`
- Production sample regression selectors: `collect_ui::tests::` and `collect_app::tests::`, with `collect-ui` enabled
- `cargo check -p orr_editor --lib`
- `cargo check -p orr_editor --no-default-features --features collect-ui --lib`
- `cargo clippy -p orr_editor --features collect-ui --lib --tests -- -D warnings`
- Explicit GPU case: `app::collect_ui_preview::tests::actual_editor_preview_gpu_matches_production_and_never_dispatches`, with `--ignored --exact --nocapture`

Default/minimal feature checks and strict Clippy passed. The existing Collect authoring panel and production runtime were not modified.

## Actual editor acceptance

The production EditorApp preview matched production `CollectUi` pixels in **seven cases**, covering Title, Menu, Playing and Terminal screens; landscape and portrait sizes; accepted unsaved edits; and saved/reopened presentation. Fourteen retained images pair each preview canvas with its full editor view.

Acceptance verifies readable default-width controls, authored labels/buttons, clipping and admitted-font rendering, observed Play/Menu/Restart/Quit actions, pointer-leave cancellation, UI Undo/Save/reopen, and resource release on close or missing panel. Preview interaction preserves the host checksum, history, mode, input state and scene manifest. Eight source/guide identities remained unchanged during final GPU acceptance; detailed evidence is retained separately.

## Original failures and corrections

- Initial compilation found an egui texture-upload API mismatch: a texture ID may carry multiple image deltas. Uploads now iterate each delta; an unused test import was removed.
- First GPU execution exposed a production texture-delta lifecycle defect. Applied deltas were still left in their container, triggering its drop assertion. The renderer now drains uploaded texture updates and frees only after submission; no assertion was suppressed.
- The next GPU run passed six pixel-parity cases, then a test clicked stale accessibility geometry after reinstalling a saved presentation. The fixture now allows layout to refresh before the next click. Pixel comparisons and host-isolation assertions were preserved.

The final CPU and GPU runs include these corrections. Original failure records remain retained.

## Scope and limits

Rendering was exercised with a software Vulkan GPU and synthetic editor interaction. This establishes actual editor rendering, production-UI pixel parity and host isolation. It does not establish native-window input, physical keyboard/mouse behavior, hardware-GPU performance or Windows execution. Preview HUD values are bounded synthetic samples, and observed actions are not dispatched into gameplay.
