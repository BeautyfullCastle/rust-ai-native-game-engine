# Frame selected collider verification — 2026-10-19

Base: `dd86142db298ff34c908b328e6f5f4de370712c3`. Local verification ran on Linux on 2026-10-18 UTC. Independent source and workflow reviews approved the scoped implementation and gates.

## Results

- **23 CPU tests passed:** six framing tests (four geometry/projection tests and two Room integration tests), plus seventeen existing Room camera capture/coherence/panel tests. Commands: `cargo test -p orr_editor --lib --features room-authoring` with filters `frame_selection` and `room_camera`, each using `--test-threads=1`.
- Default library check, minimal `--no-default-features --features models`, and minimal `--no-default-features --features room-project` checks passed. Yard3D has no separate editor Cargo feature; models is the relevant minimal framing boundary.
- Integration target built and strict library plus `frame_selection_ui` Clippy passed with `--features room-authoring -- -D warnings`.
- Actual production EditorApp **Room UI passed 1/1 in 6.25 s**, with three PNGs. Evidence: `/workspace/shared/frame-selection-ui-20261019-v1`, including `acceptance.json`, `run.log`, `source-before.sha256` and `source-stability.log`.

The actual UI used a pointer click on the header button and viewport F. Its receipt confirms full collider framing with margin, preserved yaw/pitch, unfocused and held-pointer F refusal, disabled no-selection/Play actions, and unchanged authoring Frame/history/selection/files. The session camera is the only intended changed state.

Compiler logs: `/tmp/frame-selection-v2-{cpu,camera,ui-build,default,models,room,clippy,stability}.log`.

## Reviewed fixes and retained failure

Before compilation, review identified missing models cfg guards at the viewport hooks and F invocation during an active pointer drag. The corrected shortcut rejects held pointers and drag phases. A shared blanket pointer-down guard would also disable the header button's own primary click; review caught that, and the final shared guard distinguishes viewport-origin primary drags and secondary/middle presses. The actual UI covers a real synthetic pointer click on the header and held-pointer F refusal.

Initial CPU compilation failed E0507 when the fixture moved an optional GUID out of a borrowed row. The test now clones the GUID; production code was unchanged by that fix. The original `/tmp/frame-selection-v1-cpu.log` and source manifest are retained.

## Frozen source

Final manifest `/tmp/frame-selection-v2-source.sha256` has SHA256 `5291fbf61aba25e7277705b9fd71ac4ff58fd5133ee1fcecb1e7d3543b558915`. All eight entries remained unchanged through UI execution and were rechecked afterward:

```text
a6ca829193d0cccaff3ea383712fc11615894b3282b6a44fe7b9f89975560b4f  crates/orr_editor/src/editor.rs
cedda60ec01dfd5faf7df1d1b156da6cf8527766159cbe0eca6a7692db2d7b14  crates/orr_editor/src/editor/frame_selection.rs
55fa0f3dfafee40a3c07a6905705a232e0dfb6417085e95c419d18434dd6ef46  crates/orr_editor/src/editor/frame_selection/math.rs
16849c3903e849b0f62dc172d54a66363bce092075e02417950be37e02000f0a  crates/orr_editor/src/editor/frame_selection/tests.rs
a361c88572271f7e6ce52d90650a0f45f24780ec87244b56aa138abca3f38422  crates/orr_editor/src/app.rs
d5a31229ed791ce2d44cb816a4c0b5b5165d03bbecf997ebe459c3ff5132fbbd  crates/orr_editor/src/app/frame_selection.rs
c3531524be2d5631c2afc9c319d995cb2ff5c612db15435d25c24edc658dba48  crates/orr_editor/tests/frame_selection_ui.rs
7df7a4d0b243a0e1157f1630590fff6ac7352b1be23e5a06da3716d5dbb47998  docs/editor-frame-selected-collider.md
```

## Scope and limitations

Framing uses admitted collider bounds, with conservative enclosing radii for boxes/capsules, rather than imported model bounds. CPU coverage includes perspective/orthographic landscape and portrait fitting, preserved angles, clipping/invalid/oversized refusals and Room coherence refusal without camera mutation. Yard3D shares the implementation and passed the models compile boundary, but this cohort did not execute a dedicated Yard3D UI or host integration case.

Actual UI rendering used llvmpipe (LLVM 19.1.7, 256 bits) with synthetic input in an owned Room fixture. It establishes no physical OS-input, hardware-GPU or Windows execution claim.

## Visual caveat found during screenshot review

Parent review noticed the yellow selection outline clipping in the retained screenshot. Independent UI-owner inspection confirmed that the existing overlay in `viewport3d.rs` (lines 48–59 at this revision) draws an axis-aligned **cube** with half-size `max(half_extents) * 1.08`. Thus the selected thin wall (`e_00000001`) with actual collider half-extents `[0.2, 2, 0.2]` has an overlay cube half-size of `2.16` on every axis. That oversized overlay is not the collider boundary.

The fixture/inspector identity and all eight true collider corners were checked; the true collider fits the asserted 3% viewport margin. The passing fit claim is limited to actual collider geometry and does not promise that this existing oversized selection overlay fits. The screenshot is preserved unchanged. No renderer or production change was made for this caveat. This verification amendment and the accompanying user-guide clarification were added after the UI hash cohort; the frozen hashes above describe the executed cohort, not those later documentation-only amendments.
