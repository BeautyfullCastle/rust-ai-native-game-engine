# Mesh hazard viewport picking verification — 2026-10-19

Verified locally on Linux against base `8b9678ed8f9019e345c62afacec923d61716afda`.
This adds draft-only selected-sphere center picking on the admitted walkable
surface, followed by the existing guarded Apply path. Radius and other spheres
remain unchanged. Cancellation consumes the gesture without generic selection
fallthrough. Independent source review identified and resolved an Escape
selection-clear issue using the pre-frame picking ownership latch. Final source
approval confirmed the Escape fix, staged XYZ/radius preservation, stale-context
checks and pointer ownership; the reviewer also confirmed the 22-test CPU log.
The additive workflow received independent review approval. The UI gate below
passed afterward.

## Passing checks

- **22 CPU tests passed**: 11 new viewport-picking tests and 11 existing hazard
  panel tests, serially. Command: `cargo test -p orr_editor --lib --features
  mesh-hazards-export,mesh-animated-models mesh_hazards_panel:: -- --test-threads=1`.
  Log: `/tmp/hazard-picking-v2-cpu.log`. The only warning was a pre-existing unused
  import in protected `mesh_workspace/tests.rs`.
- Minimal production library check and strict Clippy passed with
  `--no-default-features --features mesh-hazards`; Clippy used `-- -D warnings`.
  Logs: `/tmp/hazard-picking-v2-minimal-check.log` and
  `/tmp/hazard-picking-v2-clippy.log`.
- Integration test compilation passed after test-only fixture refinements:
  `/tmp/hazard-picking-v5-ui-build.log`. Production source and CPU tests remained
  byte-identical to the passing v2 checks.
- **Actual production EditorApp UI: 1 passed, 0 failed**, 26.86 seconds.
  Exact opt-in test:
  `hazard_center_viewport_pick_stages_only_selected_sphere_and_apply_undo_save_reopen`.
  Evidence: `/workspace/shared/hazard-picking-ui-20261019-v3/` containing
  `run.log`, `acceptance.json`, `source-before.sha256`, `source-stability.log`
  and six PNGs:
  `01-picking-guards.png`, `02-picked-draft.png`, `03-picked-applied.png`,
  `04-picked-reopened.png`, `05-picked-route-loss.png`, `06-picked-stop-reset.png`.

The UI gate exercised synthetic viewport pointer picking, selected-sphere-only
staging, radius preservation, unchanged host/history before Apply, Play/Save/Open
guards, Escape preserving a genuinely nonempty selection, focus-loss cancellation,
no selection fallthrough, and unchanged mission/agent state. Apply/Undo/Redo/Save/
Reopen succeeded. Playing the viewport-authored scene produced sphere-2 loss at
tick 1; Stop restored the authored checksum. Receipt checksums were
`4862235568699714076` before editing and `15916462878743660919` after Apply.

## Frozen source evidence

All six entries remained unchanged through final UI acceptance and a subsequent
checksum verification. Manifest `/tmp/hazard-picking-v5-source.sha256` has SHA-256
`8214fa27d4f8c8ecd1c0928e935a7650bf5b3f1d490c90eb1b3fe5584d652ad8`.
Paths below are relative to the repository:

| File | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/app.rs` | `3c7df44a6cca29a6849dd40ee864e15301d023329e9332750ba5e387dc0b4c67` |
| `crates/orr_editor/src/mesh_hazards_panel.rs` | `828639392c8509a2b2ed2d5ae150c8f7fa79392fe46041eb0442df9309e8d880` |
| `crates/orr_editor/src/mesh_hazards_panel/viewport_picking.rs` | `8e1e395b6888a43139b8f7ee64df2cef27981a4e32f0a99571f20ed39a8d3958` |
| `crates/orr_editor/src/mesh_hazards_panel/viewport_picking/tests.rs` | `439f669bfb890846fbd3725eed995a5016342c101986b53ab4d0183c5912698b` |
| `crates/orr_editor/tests/mesh_hazards_ui.rs` | `a4db8143f55e2355cadebb17a63c5ed931ad3a0e9ff595b10eb7703a87fa02fc` |
| `docs/editor-mesh-hazards.md` | `2573def6e9080854f9ecb8cf530dfdb25bdeac55f75f50a16cdd50551631bcf8` |

## Retained failures and limitations

- First CPU run: **21 passed, 1 failed**, `/tmp/hazard-picking-v1-cpu.log`.
  The positive fixture clicked a shared triangle boundary; floating-ray and
  fixed-point triangle admission could disagree. The fixture now targets an
  explicit triangle interior and directly verifies admitted picking. The
  production triangle-identity guard was not relaxed.
- First UI run refused fixture `world.spawn` under the MeshNavigation profile.
  `/tmp/hazard-picking-ui-v1.log` is retained. The fixture now selects the
  existing admitted agent entity, with an explicit nonempty-selection assertion.
- Second UI run passed guard/Escape checks but failed focus-loss injection:
  `Event::WindowFocused` alone did not change `RawInput.focused`. The test now
  sets the raw focus field, matching existing harness usage. Failure retained at
  `/tmp/hazard-picking-ui-v2.log`. No production change was made for either UI
  fixture correction.

UI evidence uses software Vulkan and synthetic egui/AccessKit input. It does not
establish physical GPU, physical OS input, native desktop-window or Windows
acceptance. Remote CI results and publication are separate gates. This document
was added after the frozen six-file verification; `git diff --check` is clean.
