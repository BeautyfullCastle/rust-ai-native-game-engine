# Two-corner Room wall footprint verification — 2026-10-19

Local Linux verification against base `e92fc29facc70b803b65ce04b0115247fd0d8f71`.
Two viewport clicks stage an XZ footprint; explicit Create wall remains the
single authoritative edit. Existing vertical defaults (center Y 0.5 and height
1) are preserved. Renderer, scene schema and reserved CLI behavior are unchanged.
Independent footprint, fixture/cfg and additive workflow reviews approved the
scoped changes.

## Passing checks

- **20 distinct CPU tests passed**: nine new footprint tests plus three existing
  editor stamp regressions in `/tmp/room-wall-footprint-v4-cpu.log`, and eight
  existing pure stamp tests in `/tmp/room-wall-footprint-v4-stamp.log`.
  Commands used `cargo test -p orr_editor --lib --features room-authoring` with
  selectors `room_authoring` and `room_wall_stamp::`, respectively, serially.
- Default library and minimal `--no-default-features --features room-authoring`
  checks passed: `/tmp/room-wall-footprint-v5-default.log` and
  `/tmp/room-wall-footprint-v5-minimal.log`.
- Strict library/UI Clippy passed: `cargo clippy -p orr_editor --lib --test
  room_wall_footprint_ui --features room-authoring -- -D warnings`.
  Final log: `/tmp/room-wall-footprint-v6-clippy.log`.
- Actual production EditorApp UI: **1 passed, 0 failed**, 11.55 seconds.
  Exact ignored test: `two_corner_wall_stages_creates_persists_and_blocks_player`.
  Build log: `/tmp/room-wall-footprint-v6-ui-build.log`.

Final UI evidence: `/workspace/shared/room-wall-footprint-ui-20261019-v2/`, with
`acceptance.json`, `run.log`, source-before/stability records and five PNGs:
`01-first-corner.png`, `02-staged-footprint.png`, `03-created-wall.png`,
`04-reopened-wall.png`, `05-play-collision.png`.

The gate verified draft-only two-corner staging, Escape preserving selection,
Create with a fresh wall GUID, Undo/Redo/Save/Reopen, and player collision with
the authored wall. Actors, models and camera were preserved. Core tests cover
order-independent corners, perspective/orthographic ground hits in both aspect
orientations, invalid/parallel/behind rays, nonfinite/overflow/zero/subquantum
rejection without clamping, context invalidation, Play cancellation, and one-edit
Create semantics. Review also fixed idle cancellation overwriting shared Create
status; an added regression verifies idle cancellation is a no-op.

## Retained failures and exact repairs

1. Linux libtest compilation initially failed E0433: the pre-existing stamp
   fixture called Windows-only `windows_project::create_room_project`.
   `/tmp/room-wall-footprint-v1-cpu.log` is retained. The approved test-only
   replacement creates the same kind of admitted Room using an unbaked scene,
   readable camera, installed real package and parsed PLAYER/KEY model bindings,
   then retains `PreparedProject` admission. All three existing test bodies and
   assertions from `add_source_wall` onward remain byte-identical to the base.
2. New UI compilation failed E0599 because its fixture called `FrameView.entities`.
   `/tmp/room-wall-footprint-v2-ui-build.log` is retained. Test-only code now
   obtains entity handles from admitted editor rows and reads actors via
   `FrameView::get`; production code was unchanged.
3. Strict Clippy exposed 27 existing private wall-stamp dead-code errors on Linux:
   `/tmp/room-wall-footprint-v3-clippy.log`. Existing production consumers are
   Windows x86_64 panel/store code. Three reviewed cfg-only changes align the
   private module and two crate-private methods to `any(test, windows+x86_64)`
   within room-authoring, preserving Linux unit coverage and Windows UI behavior.
   A companion BTreeSet import received the identical guard after its unused
   import failed `/tmp/room-wall-footprint-v4-clippy.log`. No lint allowances,
   public API expansion, method-body or reserved CLI changes were made.
4. First actual UI run passed staging/Create but its synthetic CTRL shortcut did
   not trigger Undo because the app expects COMMAND. The fixture now uses actual
   Edit Undo/Redo and File Save menu actions. Failure preserved at
   `/tmp/room-wall-footprint-ui-v1.log`; production code was unchanged.

The 20 CPU passes preceded only the matching import guard and UI-fixture menu
correction. Neither changes unit-test semantics; final production/UI build and
strict gates ran after both. Source review confirmed all BTreeSet uses are inside
the two identically guarded methods.

## Frozen source evidence

All ten entries stayed unchanged through final UI acceptance and subsequent
checksum verification. Manifest `/tmp/room-wall-footprint-v6-source.sha256` has
SHA-256 `8924db5904a6a7e8c4440cb8cbc553f5723b5e572040cd34ac319ed804d26930`.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/room_authoring_panel.rs` | `cdd9d22a6064e01d490e57487828a0461dcb5a49aa3be02bc4b638d7aedccf16` |
| `crates/orr_editor/src/room_authoring_panel/wall_footprint.rs` | `5ce272a84acf8b4d4c6063994e3905d8b203845af0c3cf2af96bc9ecf56d5fba` |
| `crates/orr_editor/src/room_authoring_panel/wall_footprint/tests.rs` | `0791d0b53ae96ab38a260a0dad3149e7924b41189bd52889d2223d40a1aa0f7b` |
| `crates/orr_editor/src/app.rs` | `5471c05045f353bd9b6b3dc5f627cebf9c67f193cc58e1243e820b7c7e202f3c` |
| `crates/orr_editor/src/app/room_wall_footprint.rs` | `52f4fa5dfda3dcda32a6b83d04785082f0ded13f55ef4c34035a9f1560d42687` |
| `crates/orr_editor/src/room_wall_stamp_editor_tests.rs` | `e3f17798e13adeb3bbac7997254587a8fb4947463888da7ee70b95a01e2d40aa` |
| `crates/orr_editor/src/lib.rs` | `2ba7e93ee94e20587912663a23d8a37f27fc777d2b3a018af96b7d5064ea2da1` |
| `crates/orr_editor/src/room_authoring.rs` | `bd4f5709b0964e0634262fa602b23d35b38947a06b95af2a78fe51a0bbd6aa68` |
| `crates/orr_editor/tests/room_wall_footprint_ui.rs` | `0d03f74c14a7cbdea8b455b3f5ee2fc9c06c932d647e666dc93be211e816b0e0` |
| `docs/editor-room-wall-footprint.md` | `779fea0f935116c5f00c6ae8d552167a1dfec11885262f4da530d264e56d3840` |

## Limits

UI rendering used llvmpipe (LLVM 19.1.7, 256 bits) software Vulkan with synthetic
egui/AccessKit pointer and menu input. This does not establish physical-GPU,
physical-OS-input, native desktop-window or Windows execution acceptance.
Windows CI and remote publication remain separate gates. This report was added
after source freezing; `git diff --check` is clean.
