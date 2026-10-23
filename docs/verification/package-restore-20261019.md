# Restore last package removal verification — 2026-10-19

Local Linux verification against base `55ea49c4899d53787e0d4f1d100d6e4ce3768d7e`.
A successful checked removal produces an opaque, session-local receipt. Explicit
restoration preview and commit restore the exact prior lock from validated cached
objects, including dependency closure and original direct-selection flags. The
current lock and saved Room identity/bytes must still match. This is not general
history rollback, cache discovery, or source reinstallation. Only disposable
fixtures were exercised; no actual user project was modified.

## Passing checks

- Focused core restoration: **7/7 passed**, `cargo test -p orr_package --test
  restore_removed`; `/tmp/restore-remove-core-v1-focused.log`.
- Full package suite: **93 distinct tests passed**, the prior 86 plus seven
  restoration tests. The focused seven are included in 93; the FIFO child-process
  subrun is not counted twice. `/tmp/restore-remove-core-v1-all.log`.
- Strict package Clippy passed: `cargo clippy -p orr_package --all-targets --
  -D warnings`; `/tmp/restore-remove-core-v1-clippy.log`.
- Editor: **35/35 passed**, comprising 22 removal/restoration model tests and
  13 panel tests. `cargo test -p orr_editor --lib --features
  package-manager,room-animated-models package_manager -- --test-threads=1`;
  `/tmp/restore-remove-editor-v3-cpu.log`.
- Minimal library check passed with `--no-default-features --features
  package-manager`; strict library/UI-test Clippy passed with `--lib --test
  package_removal_ui --features package-manager,room-animated-models -- -D warnings`.
  Logs: `/tmp/restore-remove-editor-v3-minimal.log` and
  `/tmp/restore-remove-editor-v3-clippy.log`.
- Actual production EditorApp UI: **1 passed, 0 failed**, 17.62 seconds.
  Exact ignored test:
  `verified_room_package_removal_preserves_content_and_reopens_for_play`.
  Build log: `/tmp/restore-remove-editor-v3-ui-build.log`.

Independent core, editor/UI and additive workflow reviews approved the changes.
Remote CI and publication remain separate gates.

## Actual UI evidence

`/workspace/shared/package-restore-ui-20261019-v1/` contains `acceptance.json`,
`run.log`, `source-before.sha256`, `source-stability.log`, and nine PNGs:
`01-verified.png`, `02-used-refused.png`, `03-removal-preview.png`,
`04-inventory-refreshed.png`, `05-restoration-preview.png`, `06-restored.png`,
`07-restored-model-assigned.png`, `08-reopened.png`, `09-playing.png`.

The gate retains removal coverage (used static/animated refusal, stale-lock
refusal, retained dependency, explicit confirmation, unchanged content/checksum/
history) and adds exact prior-lock restoration including transitive dependencies
and direct flags. Original fixture source directories were unavailable during
restore and Reopen. A restored model was assigned, saved, reopened and played;
source directories were restored afterward. This proves cache-only recovery in
that bounded fixture, without requiring the original source package paths.

## Review correction and retained failure

Review identified that fresh Verify needed to invalidate the last-removal
receipt and restoration preview/generation. Idle Verify now discards them before
worker dispatch. The regression uses a real successful opaque receipt and reverse
preview, checks immediate invalidation, and checks completion does not revive it.
Existing mutating-worker authoring freeze is reused through the completion frame
for both removal and restoration.

The first run of that regression failed admission because its fixture had no
real Room model binding: **34 passed, 1 failed** at the required-binding check.
The fixture now installs the shipped static package and binds its admitted model
to the actual Room player GUID. No admission guard was relaxed. Failure retained
at `/tmp/restore-remove-editor-v2-cpu.log`; the earlier 34-test v1 cohort remains
interim evidence. Final v3 passed all 35 without warnings. The final actual UI run
passed on its first restoration attempt.

## Frozen source evidence

All nine entries stayed unchanged through final UI acceptance and subsequent
checksum verification. Manifest `/tmp/restore-remove-editor-v3-source.sha256`
has SHA-256 `7defd5e331b58ab2258234773961e3b228a2fb85706d94caa340e21b10fc1f6c`.
Core source remained identical to the passing 93-test/all-target Clippy cohort.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_package/src/lib.rs` | `a585d1d03f175b2abd6ce6e167b06e7836b10c102d277b51e31306618187a0b4` |
| `crates/orr_package/src/restore_removed.rs` | `7ace5acd691f7b3a806c8bcd87edb63bdc46278f543b54ae5f07ee4eb65d81c8` |
| `crates/orr_package/tests/restore_removed.rs` | `520945099c4696c6862675f2a3fd0a0792e7608dac51c7f9ff0fc13521051718` |
| `crates/orr_editor/src/app.rs` | `dc09228e0ab3eea3a1aefdae5f4869404536356658f717fbf1079e9645193cb3` |
| `crates/orr_editor/src/package_manager.rs` | `7bdc40d2e2f9e5ad7e7296db3b0728fdf7e901a1db511a29a7aac878df11cbc0` |
| `crates/orr_editor/src/package_manager/removal.rs` | `ba5bd866285aeecad903b97a92b2dfe46922f4b6a6cde1dbb5fe1baf33079bbd` |
| `crates/orr_editor/src/package_manager_panel.rs` | `bb53a1a34687551b473ac2f57fcdc65cc37338ed0983cb9bbb90ea1e4739d8eb` |
| `crates/orr_editor/tests/package_removal_ui.rs` | `c54dce180b351d574b1849230c1cc8eacf3be0ad6ea15ed56df2ca0846b69569` |
| `docs/editor-content-packages.md` | `b3f9eaa0f11769cef7bcc077c073a4e71e2bf9d64494fbcc0461e96f85ecf404` |

## Limits

UI evidence uses synthetic egui/AccessKit input and llvmpipe (LLVM 19.1.7,
256 bits) software Vulkan. It does not verify physical GPU, physical OS input,
native desktop-window or Windows execution. Non-Unix root identity relies on
creation timestamps rather than Unix device/inode identity; Windows timestamp/
attribute comparisons are not a claim of file-ID-equivalent race isolation.

The package writer guard and editor authoring freeze do not isolate arbitrary
external host/ERP or filesystem writers. Exact saved-byte/lock validation is
fail-closed at checked boundaries, not a cross-process filesystem transaction.
The receipt is ephemeral and intentionally invalidated by fresh verification or
context changes; it is not a persistent undo history. This report was added after
source freezing; `git diff --check` is clean.
