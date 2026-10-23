# Static package model preview verification — 2026-10-19

Base: `d43650c5e1246bf0bc879844c24f8e5ff9350fde`. Local verification ran on Linux on 2026-10-18 UTC. Root visually inspected foreground/background previews and corrupt-asset refusal. Independent source review approved the final implementation at 21:53:55 UTC; independent workflow review approved its exact gates and receipt predicates at 21:56:09 UTC (workflow SHA prefix `59025d29`).

## Results

- **55 CPU tests passed:** twenty preview admission/state/camera tests with `--features package-manager`, and thirty-five existing package panel/removal/restore tests with `--features package-manager,room-animated-models`. Filters: `package_model_preview::` and `package_manager`, serially.
- Default library and minimal `--no-default-features --features package-manager` checks passed. Strict library plus `package_model_preview_ui` Clippy passed with `--features package-manager -- -D warnings`.
- **New preview UI passed 1/1 in 15.38 s**, with fourteen full/cropped PNGs: `/workspace/shared/package-preview-ui-20261019-v1`. Receipt records 38,325 different-asset pixels, 24,195 orbit-changed pixels, 37,433 zoom-changed pixels and exact Reset. Preview without a selected entity, unchanged project/source files, Frame/history/bindings/editor camera, corrupt cached-asset refusal and separate Use Model/Save/reopen all passed.
- **Existing combined removal/restore UI passed 1/1 in 20.21 s**, rebuilt with `package-manager,room-animated-models`, with nine PNGs: `/workspace/shared/package-preview-removal-ui-20261019-v1`. It covers used static/animated refusal, stale lock refusal, exact prior-lock/transitive-dependency restoration while sources are hidden, restored model assignment/Save/reopen/Play, and preserved cache/source/scene/model/camera/history invariants.

Both directories include receipts, run logs, source manifest and source-stability proof. Compiler logs: `/tmp/package-preview-v1-{cpu,package,ui-build,default,minimal}.log`, `/tmp/package-preview-v2-{ui-build,clippy,removal-build}.log`. Actual logs: `/tmp/package-preview-v1-{ui,removal-ui}.log`.

## Retained failure and resource handling

Initial strict integration Clippy rejected two constant-size `chunks_exact(4)` calls in the pixel-comparison test. Both were mechanically replaced with `as_chunks::<4>().0.iter()`, without lint allowances or production changes. Original `/tmp/package-preview-v1-clippy.log` and source manifest are retained. Both actual UI cases passed on their first execution.

Disk fell below the agreed link threshold after the first UI. Further links paused while root removed only inactive incremental cache and older regenerable library-test executables; sources, logs, evidence and accepted runtimes were preserved. The regression resumed after space recovery and passed.

## Frozen source

Manifest `/tmp/package-preview-v2-source.sha256` SHA256: `31e6fe7cb5285c2fcd44e7f83ec845e42812f50cf4254f41a5a55338096592d2`. All ten entries remained unchanged through both actual UI cases:

```text
4dd53d86b95f267cfcceafa8633bb92463355fdd810c1b5c7760d0cbb9c9ef6a  crates/orr_editor/src/app.rs
fc4ee4ac1e7806798808c7d288a0abd714e6904201d6fd4125d2b03a0244c333  crates/orr_editor/src/app/package_model_preview.rs
6fbfedf8060a09b9e5109b7a9b0b18206b12a50ac197b8e417f6db340d5eff3d  crates/orr_editor/src/lib.rs
185b44d1b20a02d5b524bcc8a8100e3d74b83c917bf447f71080279d60bc755f  crates/orr_editor/src/package_manager_panel.rs
1544bb69dd1edc2d8fd244cdb05805b16f526e698e8fea17a88139c5b5ba1800  crates/orr_editor/src/package_model_preview.rs
359c8fe32816181c5597b4911b4ac8ecbe29f37cf6bf363c73eff6b1adf93b59  crates/orr_editor/src/package_model_preview/camera.rs
82b33dff5d59307f8beecda64d9c2c061bcb10077b56e3245ebba86d0624c83a  crates/orr_editor/src/package_model_preview/tests.rs
348d6ff3305ac409be0ca38543b46f64e3a0994e64ba9c46f4d14d68c17f8927  crates/orr_editor/tests/package_model_preview_ui.rs
80e2cfd7c0f3df3475038f3cafd0fbdf733d5708d0e185dfe5a29694df05230f  docs/editor-package-preview.md
38af6f6e7dd9be7069c5cc9004b18ef7785f274b199d3b30ecff3d0f78a2db9d  docs/editor-package-search.md

```

## Scope

These are static previews using verified installed package bytes and an independent preview camera; preview itself does not assign a model. The separate explicit Use Model path was exercised afterward. The legacy `package_manager_ui` target is Windows-x86_64 gated and was not counted as a Linux pass; local assignment evidence comes from the new preview UI and combined restoration regression.

Both actual cases used production EditorApp with llvmpipe (LLVM 19.1.7, 256 bits) and synthetic input in owned fixtures. No physical OS-input, hardware-GPU or Windows execution claim is made. The expected Windows core inventory is nineteen tests because one symlink test is Unix-only; that count is source inventory, not an executed Windows result.

Filesystem scope: the trusted project/cache directory assumption remains. Verification and delivery checks do not claim isolation against arbitrary concurrent hostile directory replacement or eliminate every filesystem time-of-check/time-of-use race.
