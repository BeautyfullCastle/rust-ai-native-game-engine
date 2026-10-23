# Animated package preview verification — 2026-10-19

Base: `0f4181ca71a999b878404c007eaa548a4f5cf652`. Local Linux verification ran on 2026-10-18 UTC. Independent source review approved the frozen implementation.

## Results

The main animated cohort totals **69 distinct tests** (33 preview + 36 package). The 21-test feature boundary and one exact unified-feature refusal below overlap and are reported separately. Root visually inspected sampled poses and the invalid-input retained canvas.

- **33 animated preview CPU tests passed** with `package-manager,room-animated-models`: thirteen new animation tests plus twenty static/state/camera baseline tests.
- **36 package panel/removal/restore regression tests passed** under the same features.
- **21 static/no-Room-animation tests passed** with `package-manager`. These overlap the twenty baseline tests above and are a separate feature-boundary run, not twenty-one additional distinct tests.
- **One exact capability-refusal test passed** with `package-manager,animated-models`, confirming unified lower-level animation support does not implicitly enable the Room animated-preview route.
- Default library, minimal package-manager and minimal room-animated-models checks passed. Strict library plus animated/static UI Clippy passed with `--features package-manager,room-animated-models -- -D warnings`.
- **Animated actual UI passed 1/1 in 22.07 s**, nineteen full/cropped PNGs: `/workspace/shared/package-animated-preview-ui-20261019-v1`.
- **Static preview/assignment actual UI passed 1/1 in 15.69 s**, fourteen full/cropped PNGs: `/workspace/shared/package-animation-static-ui-20261019-v1`.

Both actual cases used production EditorApp. The animated receipt records clip index 1 (`pulse`), explicit samples at 0.0 and 0.5 seconds, 7,135 changed pixels between samples and 2,136 orbit-changed pixels. Unsampled drafts leave the pose unchanged; accepted samples remain held without live playback; invalid indices/times retain the exact prior pose. Reset, replacement/close reset, preview without selection and unchanged files/Frame/history/bindings/editor camera passed. The separate animated Use Model action retained clip-zero assignment through Save/reopen. Static preview/assignment regression also passed unchanged.

Both evidence directories contain `receipt.json`, `run.log`, screenshots, `source-before.sha256` and `source-stability.log`. Logs: `/tmp/package-animation-v1-{cpu,package,static,unified,default,minimal,room,ui-build,clippy,ui,static-ui,stability}.log`.

## Failures and resource handling

No compile, CPU, Clippy or actual UI failures occurred in this cohort. Builds paused for disk recovery before further links; root removed only inactive incremental caches and named obsolete regenerable executables, preserving current artifacts, source and acceptance evidence. No source modification or relaxed assertion was needed to obtain the passing results.

## Frozen source

Manifest `/tmp/package-animation-v1-source.sha256` SHA256: `c98d0c784d28157b54b14b3db64fb7adc4f558993251e33a04f250d7336d52c4`. All ten entries remained unchanged through both actual UI executions:

```text
022495e8f9a3d314a82e923aa2d7a0efd173256422011bf7b68c5535fab890bd  crates/orr_editor/src/app/package_model_preview.rs
d05e51b2f0ff0d6f36cc8a9bd8528d4f9ee202a9331804384a17c20f542552b9  crates/orr_editor/src/package_manager_panel.rs
6f2efc401c2bfe581cbe7cc51a426ebd296f96468933af42d0d78dc7b21c3d80  crates/orr_editor/src/package_model_preview.rs
c2b4a974b40b732eb2bacb5317732e55ca2c3bc4b877cb8dcf2b6bacbb8b130c  crates/orr_editor/src/package_model_preview/animation.rs
7c2a4d54ca5d3b15f16b7d9fa8e23ca8fa597f66a9f6db4edeb621a9989dc9a7  crates/orr_editor/src/package_model_preview/animation/tests.rs
11b1ce52c940a2d083466f37f293f4b74cde9abbf635fef2e02dde5a05f180ac  crates/orr_editor/src/package_model_preview/camera.rs
e34ecce9f804d0b30eba289794b5e4ee9f102064f3fccef071cf8c05e4c12946  crates/orr_editor/src/package_model_preview/tests.rs
1bd5887821454bf0c8af1eb47fba185f5296e787bd43dc0dd8e8bd0a292e6af2  crates/orr_editor/tests/package_animated_preview_ui.rs
6980ba398879f660b4b1fd1ac2e150f6eaa86d6801ba2d05994c586787d5f3da  docs/editor-package-animation-preview.md
10957566e661f10ba7b7ed71f4dde0b6a485dfbe5bdd52360a671bce4f92aaff  docs/editor-package-preview.md

```

## Limits

Manual clip/time sampling is preview-only; no live animation transport or authoring-time ticking was established. Actual UI used llvmpipe (LLVM 19.1.7, 256 bits) with synthetic input and owned fixtures. These local results make no hardware-GPU, physical OS-input or Windows execution claim. Expected Windows test inventory excludes the Unix-only symlink test; it is not an executed result.

The trusted project/cache directory assumption remains. Verification and delivery checks do not provide isolation from arbitrary hostile concurrent filesystem directory replacement or eliminate every time-of-check/time-of-use race.
