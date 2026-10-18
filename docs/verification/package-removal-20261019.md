# Checked package removal verification — 2026-10-19

Local Linux verification against base `18f57e6655c5db481f239b673d56e591a7cb69c4`.
Removal requires an exact approved preview and unchanged previous/candidate locks,
then revalidates saved Room references under the existing package writer guard.
It removes a direct selection from active authority; cached objects and source
files remain available. All exercised projects were disposable test fixtures.
No actual user project or package content was deleted.

## Passing checks

- Core focused checked-removal tests: **8/8 passed**.
  `cargo test -p orr_package --test checked_remove`;
  `/tmp/checked-remove-core-v2-focused.log`.
- Full `orr_package` suite: **86 distinct tests passed**: 29 unit, 16 cached
  revision, 8 checked removal, 4 explanation, 16 verification and 13 preview.
  The FIFO child-process subrun is not counted twice. The focused eight are
  included in these 86, not additional distinct tests.
  `/tmp/checked-remove-core-v2-all.log`.
- Strict package Clippy passed with `--all-targets -- -D warnings`:
  `/tmp/checked-remove-core-v2-clippy.log`.
- Editor: **25/25 passed**, comprising 14 removal-model tests and 11 panel tests,
  including the deterministic held-worker authoring-freeze regression.
  `cargo test -p orr_editor --lib --features package-manager,room-animated-models
  package_manager -- --test-threads=1`; `/tmp/checked-remove-editor-v3-cpu.log`.
- Minimal `--no-default-features --features package-manager` library check and
  strict combined library/UI-test Clippy passed. Clippy selected `--lib --test
  package_removal_ui --features package-manager,room-animated-models -- -D warnings`.
  Logs: `/tmp/checked-remove-editor-v3-minimal.log` and
  `/tmp/checked-remove-editor-v3-clippy.log`.
- Actual production EditorApp UI: **1 passed, 0 failed**, 12.23 seconds.
  Exact ignored test:
  `verified_room_package_removal_preserves_content_and_reopens_for_play`.
  Build log: `/tmp/checked-remove-editor-v3-ui-build.log`.

Final UI evidence: `/workspace/shared/package-removal-ui-20261019-v2/`, containing
`run.log`, `acceptance.json`, `source-before.sha256`, `source-stability.log`, and
six PNGs: `01-verified.png`, `02-used-refused.png`, `03-removal-preview.png`,
`04-inventory-refreshed.png`, `05-reopened.png`, `06-playing.png`.
The receipt confirms static/animated used-content refusal, stale-lock refusal,
direct-to-retained-dependency behavior, explicit Preview/Remove, preservation of
cache/source/scene/models/camera bytes and checksum/history, refreshed inventory,
and Reopen/Play/Stop.

## Review fixes and retained failures

Independent source review required bounded no-follow/nonblocking Room-file reads
and project-root identity checks. Deterministic tests cover replacement by FIFO
or symlink without blocking the writer, stale saved bytes and replaced roots.
Review also required authoring to remain frozen during mutating removal through
the completion frame, including direct Save/Play/Step guards. The pending return
now precedes same-frame model-request dispatch, and starting removal clears queued
model requests. Final editor gates ran after these ordering fixes. Additive
workflow review was approved before final UI completion; remote CI is separate.

The first core run had **5 passes and 3 failures** because dependency fixtures
used `=1.0.0` where the API requires exact `1.0.0`. Only fixture strings changed;
the version parser was not relaxed. Original log:
`/tmp/checked-remove-core-v1-focused.log`.

The first UI run correctly rejected a fixture source containing uncanonicalized
parent traversal. The fixture now canonicalizes the repository asset source.
Failure and stable-source proof remain at
`/workspace/shared/package-removal-ui-20261019-v1/` and
`/tmp/package-removal-ui-v1.log`. Interim v1/v2 editor results remain preserved;
v3 is the final production-source cohort. An unused test-only import warning in
v2 was removed before the warning-free final run.

## Frozen source evidence

All nine entries remained unchanged through final UI acceptance and subsequent
checksum verification. Manifest `/tmp/checked-remove-editor-v3-source.sha256`
has SHA-256 `32058d58363dcfc407a952b328b97d0b2fc0deea5bfca8ecd7f5e5bdb1e8d7b3`.
Core source stayed identical to the passing full-package/Clippy cohort.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_package/src/lib.rs` | `7c49f937e73f94c84a2c1e37d0c9474eb417983b109eab54c04815ccf7f7a714` |
| `crates/orr_package/src/checked_remove.rs` | `95dff5e1168d31a74da086e81f950794196bd896cba160eb005cbf02d65aa17f` |
| `crates/orr_package/tests/checked_remove.rs` | `4b21d5d2488c41f6d8e8991930bc2aaf1d1d8d7ec5fb284699646952666051d2` |
| `crates/orr_editor/src/app.rs` | `86b77c26d7c62807e5ef06845057fcc64fd555e44c19192a5470c05e72451899` |
| `crates/orr_editor/src/package_manager.rs` | `c6cb2b6e818efb3baa398079e6fbaf62ed82c032ea893fc3d8b215425232d0c4` |
| `crates/orr_editor/src/package_manager/removal.rs` | `eaae551536a2ff0c4fb1d2ab0ef46a062445b97fab6178c827f062efceeafb20` |
| `crates/orr_editor/src/package_manager_panel.rs` | `f2a800af293d70033573308c61caa2747ab046300165cfa9857bca663e89cea3` |
| `crates/orr_editor/tests/package_removal_ui.rs` | `5fb62a318b998a1025d010cbebbfc257d83c0eae981f5509d917e482f3ad7a11` |
| `docs/editor-content-packages.md` | `8d96f929fdc66385c73cf1f6ba85cf23a5d20ae74c2b74b43e59bc2daf7a8dbd` |

## Limits

The UI used synthetic egui/AccessKit input and llvmpipe (LLVM 19.1.7, 256 bits)
software Vulkan. Physical GPU, physical OS input, native desktop-window and
Windows execution are unverified. Unix root identity uses device/inode;
non-Unix root identity uses creation timestamp, and Windows file comparisons use
creation time/attributes plus size/modification time. This is not a claim of
Windows file-ID-equivalent replacement isolation.

The UI authoring freeze and package writer guard do not isolate arbitrary
external host/ERP or filesystem writers. Saved-byte revalidation is fail-closed
at the checked boundaries, not a general filesystem transaction across all
external processes. Source review and fixture acceptance do not remove this
limitation. This report was added after source freezing; `git diff --check` is
clean.
