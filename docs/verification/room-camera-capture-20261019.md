# Room viewport camera capture verification — 2026-10-19

Local Linux verification against base `ab43db2340981bcb3b995d61104a47d5b9f1f2af`.
The additive capture flow stages the current viewport camera, permits Cancel or
explicit Apply, and provides bounded last-capture Undo/Redo. It does not extend
generic scene history. Independent source and additive workflow reviews approved
the changes. Reserved `room_camera_panel.rs` and sample `room_camera.rs` remained
unchanged; their existing validation/save behavior is reused.

## Passing checks

- **17 CPU tests passed**: six conversion/staging tests, one editor coherence
  regression, and ten existing Room camera panel tests. Command: `cargo test -p
  orr_editor --lib --features room-project room_camera -- --test-threads=1`.
  Log: `/tmp/room-camera-capture-v4-cpu.log`.
- Default library check passed: `cargo check -p orr_editor --lib`.
  `/tmp/room-camera-capture-v4-default.log`.
- Minimal Room feature check passed: `cargo check -p orr_editor --lib
  --no-default-features --features room-project`.
  `/tmp/room-camera-capture-v4-minimal.log`.
- Strict production library/UI-test Clippy passed: `cargo clippy -p orr_editor
  --lib --test room_camera_capture_ui --features room-project -- -D warnings`.
  `/tmp/room-camera-capture-v4-clippy.log`.
- Actual production EditorApp UI: **1 passed, 0 failed**, 6.67 seconds.
  Exact ignored test:
  `viewport_camera_stage_apply_cancel_history_save_reopen_and_play`.
  Build log: `/tmp/room-camera-capture-v4-ui-build.log`.

Final evidence: `/workspace/shared/room-camera-capture-ui-20261019-v2/`, including
`acceptance.json`, `run.log`, source hashes/stability log and nine PNGs:
`01-authored.png`, `02-session-navigation.png`, `03-staged-controls.png`,
`04-applied-same-framing.png`, `05-capture-undone.png`, `06-capture-redone.png`,
`07-reopened-camera.png`, `08-play-authored-camera.png`, `09-stop-authored-camera.png`.

The UI gate drove viewport orbit/pan/zoom through synthetic input events, checked
Stage/Cancel caused no authoring mutation, and verified exact viewport pixel
parity on Apply. It exercised capture-specific Undo/Redo, external-camera-change
history invalidation, Save/Reopen/Play/Stop, and unchanged scene/model/lock/generic
history state. Core tests also cover perspective matrices/rays/FOV, orthographic
zoom normalization for landscape/portrait, invalid values without clamping and
stale-context invalidation.

## Retained failures and fixes

1. Initial minimal Room compile failed E0615 because only the scene-load epoch
   accessor had its feature guard widened. The field, generator, initializer and
   load increment received the same approved `room-project` guard extension.
   Original log: `/tmp/room-camera-capture-v1-cpu.log`.
2. The corrected 16-test CPU cohort passed, then UI linking failed with signal 7
   (Bus error). Subsequent shell startup failed with `No space left on device`,
   confirming exhausted workspace storage. Original link log:
   `/tmp/room-camera-capture-v2-ui-build.log`. Recovery removed inactive incremental
   cache, obsolete regenerable editor test executables and failed link output;
   source, evidence, compiled libraries and accepted standalone runtimes were
   preserved. Cleanup ledger: `/tmp/engine-build-cache-cleanup-20261019.json`.
   Exact eight-file hashes matched after recovery; unchanged-source link and
   boundary/Clippy retries passed in the v3 logs.
3. First actual UI run revealed a production coherence bug: periodic read polls
   cleared staged capture, disabling Cancel during otherwise idle frames.
   `/tmp/room-camera-capture-ui-v1.log` and the v1 UI evidence remain preserved.
   A narrow Room capture-coherence accessor now allows read-only polling while
   rejecting mutating requests, non-Edit state and input phases other than Off.
   The existing screenshot waiting predicate is unchanged. Independent review
   checked the pending-request match and explicit input-Off fence. The new
   regression covers polling stability, mutation refusal and active input with
   no pending request. Final v4 checks and UI v2 passed after this fix.

## Frozen source evidence

All eight entries remained unchanged through final UI acceptance and subsequent
checksum verification. Manifest `/tmp/room-camera-capture-v4-source.sha256` has
SHA-256 `febe837dff50b11415d0fa7768e5674d6b8851d22cd4862f62ffd65e95adce39`.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/room_camera_capture.rs` | `f2873a9c7637ad2df84674b8630a04b27978f26a9fb4c980a8d692a499190377` |
| `crates/orr_editor/src/room_camera_capture/tests.rs` | `802e34c1ab5c0f4c6f51d3219e35928fd686ea6bbe848fc9e10436d91ebb4ec6` |
| `crates/orr_editor/src/app/room_camera_capture.rs` | `f4ca63a38ead3a1ce1eb9e2ba7b41fc5bbed35ee08fc189bec5ef9e4028e660f` |
| `crates/orr_editor/src/app.rs` | `73bd57462b616528ddfbfe2e6a1a6d33c84ca59b95d5664bf046191e1c3bcded` |
| `crates/orr_editor/src/lib.rs` | `906f133861a0d01bb7e8c22c07018fb13b8e71efe93539a3c47a2a9c157cb637` |
| `crates/orr_editor/src/editor.rs` | `04884e31660c371eb4b6e869e97e9e6240b3bcb77acdc1e5276e49afc9fca2c5` |
| `crates/orr_editor/tests/room_camera_capture_ui.rs` | `f598b2ffc9599bc75e682bf97b6a3e4d30318473caa6600a04ac2c57f90c17a3` |
| `docs/editor-room-camera-capture.md` | `56df817672a5b90c48a9729cdd35b81eecacec6acd75f842cc442d244df7a10d` |

## Limits

Rendering used llvmpipe (LLVM 19.1.7, 256 bits) software Vulkan with synthetic
egui/AccessKit input. Viewport-event coverage does not establish physical mouse/
keyboard, physical GPU or native desktop-window acceptance. Windows execution
and remote CI remain pending separate gates. This report was added after the
source freeze; `git diff --check` is clean.
