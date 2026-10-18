# Animation candidate preview verification — 2026-10-19

Local Linux verification against base `78e69a551f90e0227a6533ab3cf2686e40edfafc`.
Loaded animation candidates can be auditioned before Apply, using indexed clip
choices with names and duration. Preview remains presentation-only; existing
rights acknowledgement and authoritative Apply/history boundaries remain intact.
Independent code and additive workflow reviews approved this change.

## Passing checks

- **17 CPU tests passed**, serially: six added tests (five candidate lifecycle
  tests and one duplicate/empty clip-name test), plus eleven existing panel and
  preview tests. Command: `cargo test -p orr_editor --lib --features
  mesh-mission,mesh-animated-models mesh_animation_panel:: -- --test-threads=1`.
  Log: `/tmp/animation-candidate-v1-cpu.log`. Only the existing protected
  `mesh_workspace/tests.rs` unused-import warning appeared.
- Minimal production library checks passed with `--no-default-features` for
  both `mesh-navigation` alone and `mesh-animated-models`.
  Logs: `/tmp/animation-candidate-v1-boundary-check.log` and
  `/tmp/animation-candidate-v1-minimal-check.log`.
- Strict production library Clippy passed with `--no-default-features
  --features mesh-animated-models -- -D warnings`:
  `/tmp/animation-candidate-v1-clippy.log`.
- Strict integration-test Clippy passed: `cargo clippy -p orr_editor --test
  mesh_animation_candidate_ui --features mesh-animated-models -- -D warnings`.
  Log: `/tmp/animation-candidate-v3-ui-clippy.log`.
- **Actual production EditorApp UI: 1 passed, 0 failed**, 58.55 seconds.
  Exact opt-in test:
  `loaded_candidate_named_clip_previews_without_applying_and_preserves_authoring`.
  Build log: `/tmp/animation-candidate-v3-ui-build.log`.

UI evidence is at `/workspace/shared/animation-candidate-ui-20261019-v2/`:
`run.log`, `acceptance.json`, `source-before.sha256`, `source-stability.log`, and
13 PNGs covering baseline, loaded candidate, pulse at ticks 0 and 16, invalid
input retention, Clear, replacement, Apply, Undo, Redo, Reopen, applied preview,
and Play/Stop clearing. Named nonzero clip `pulse` changed **3,081 pixels** between
ticks 0 and 16. The gate verified unchanged Frame/history/files before Apply,
invalid clip/tick retaining the accepted pose, Clear restoration, replacement
invalidation, Apply/Undo/Redo/Save/Reopen, Play/Stop clearing and required rights
acknowledgement. Saved scene SHA-256:
`7e2040bf8bc1774cfddd4398e723eba49cefcdbbecb5a06c794266510f606f19`.

## Frozen source evidence

All eight entries remained unchanged through the final UI run and subsequent
strict integration Clippy/checksum verification. Production and CPU-test sources
were unchanged from the 17-test run. Manifest
`/tmp/animation-candidate-v3-source.sha256` has SHA-256
`d18d3fd4ce90aa1daaaeec2d60f03151867878422a083279c8428343277028e9`.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/mesh_animation_panel.rs` | `9e56d67fcb3733f69cea1f26f48563ab47a8c7bfb35aa2f70ff97940e6a9fd01` |
| `crates/orr_editor/src/mesh_animation_panel/preview.rs` | `5ac5922f948516f1e52b7a068dc304b2d37dbf3c1bbc1a1ce4892a275ba4278f` |
| `crates/orr_editor/src/mesh_animation_panel/preview/tests.rs` | `8e6c8beeb21c68fad1cbb529696276128bf70a85ddb5bfbb06b33f8713c73620` |
| `crates/orr_editor/src/mesh_animation_panel/clip_choices.rs` | `3708a0b09af4c42330cea80a0ccfc231191077e0587da24b2c8bc53388356154` |
| `crates/orr_editor/src/mesh_animation_panel/clip_choices/tests.rs` | `e17defc59000c63130db4bfd7ae245fc8373d2fce62f38c4b9917bcbc93ee351` |
| `crates/orr_editor/tests/mesh_animation_candidate_ui.rs` | `5a4d0eff445acf504a45123fb8d81e96da86b7b318b018c9cd22630696de62cd` |
| `docs/editor-mesh-animation.md` | `c41990a7f2039d23c6b82f0edda2519c1127772d6e3a7b04e5e1ada8170be4f9` |
| `docs/editor-mesh-animation-preview.md` | `0ca00469c762ac3d29bb171f392d0ef81f8b18317ffeda48c00f5da5ae323176` |

## Retained failure and scope

The first UI run failed its replacement assertion because the fixture attempted
a second Load without renewed rights acknowledgement. Load correctly remained
disabled and preserved the preview. Earlier assertions had already verified named
pulse poses, invalid-input retention, unchanged Frame/history/files and Clear.
The test now explicitly re-acknowledges rights and asserts Load is enabled before
replacement. No production behavior was changed to make this pass. Original
failure and stable-source proof remain in
`/workspace/shared/animation-candidate-ui-20261019-v1/` and
`/tmp/animation-candidate-ui-v1.log`.

The UI ran through synthetic egui/AccessKit input with llvmpipe (LLVM 19.1.7,
256 bits) software Vulkan. This establishes no physical-GPU, physical-OS-input,
native desktop-window or Windows execution claim. Remote CI/publication remain
separate gates. This verification document was added after source freezing;
`git diff --check` is clean.
