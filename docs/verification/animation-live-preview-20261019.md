# Live animation audition verification — 2026-10-19

Local Linux verification against base `29fd0863463585d08b1a5f44feea75ec6d9a0657`.
Live audition advances presentation-only animation preview without starting the
host simulation or applying candidate settings. Play/Pause, Restart, speed and
Loop controls preserve the existing static/manual preview boundary. Manual tick
bounds remain unchanged; accepted live ticks use checked u64 accumulation.
Independent source and additive workflow reviews approved the changes.

## Passing checks

- **26 CPU tests passed**, serially: prior 17 plus nine live-transport tests
  (five pure clock tests and four preview integration tests).
  `cargo test -p orr_editor --lib --features mesh-mission,mesh-animated-models
  mesh_animation_panel:: -- --test-threads=1`;
  `/tmp/animation-live-v2-cpu.log`.
  Only the pre-existing protected `mesh_workspace/tests.rs` unused-import warning
  appeared in this library-test configuration.
- Default library check, navigation-only boundary and minimal animated-feature
  checks passed. Logs: `/tmp/animation-live-v2-default.log`,
  `/tmp/animation-live-v2-boundary.log`, `/tmp/animation-live-v2-minimal.log`.
  Boundary/minimal commands used `--no-default-features` with `mesh-navigation`
  and `mesh-animated-models`, respectively.
- Strict production library/UI-test Clippy passed with `--lib --test
  mesh_animation_candidate_ui --features mesh-animated-models -- -D warnings`.
  `/tmp/animation-live-v2-clippy.log`.
- Integration build passed: `/tmp/animation-live-v2-ui-build.log`.
- **Live actual UI: 1 passed, 0 failed**, 50.22 seconds, 15 PNGs.
- **Static regression UI: 1 passed, 0 failed**, 60.07 seconds, 13 PNGs.

## Separate actual UI evidence

Live: `/workspace/shared/animation-live-ui-20261019-v1/`, exact test
`live_candidate_audition_advances_pauses_and_preserves_authoring`, with
`acceptance.json`, `run.log`, source-before/stability records and 15 captures.
Named nonzero candidate clip `pulse` changed **3,891 pixels** while playing.
Pause froze the rendered pose exactly; Restart restored tick zero, including
preserving paused state. Speed/Loop controls, focus-loss pausing, invalid-input
retention and Clear restoration passed. Candidate settings, Frame, history and
saved files stayed unchanged. Loop was explicitly enabled by the test before
asserting continued playback beyond the clip duration.

Static: `/workspace/shared/animation-live-static-ui-20261019-v1/`, exact test
`loaded_candidate_named_clip_previews_without_applying_and_preserves_authoring`,
with its own receipt/log/source proof and 13 captures. It confirms the original
manual pulse preview still changes 3,081 pixels, preserves Frame/history/files
before Apply, retains accepted pose on invalid tick/clip, clears on replacement,
and supports Apply/Undo/Redo/Save/Reopen and Play/Stop clearing with required rights
acknowledgement.

CPU tests separately cover finite monotonic time, duplicate egui passes,
backward/invalid time, missing frames and suspension, focus loss without catch-up,
fractional speed continuity, checked overflow, zero-duration clips, sampler-aligned
single-pass endpoints and unmodded loop ticks. Context, candidate-generation and
Play transitions invalidate transport appropriately.

## Retained failure

The initial compile failed E0583 because `mod transport` under an existing
path-attributed preview module resolved to the wrong directory. Explicit paths
were added for `preview/transport.rs` and `transport/tests.rs`; no transport
semantics or test expectations changed. Original log and source manifest remain
at `/tmp/animation-live-v1-cpu.log` and `/tmp/animation-live-v1-source.sha256`.
Both final actual UI cases passed on their first frozen execution.

## Frozen source evidence

All six entries remained unchanged across both final UI runs and subsequent
checksum verification. Manifest `/tmp/animation-live-v2-source.sha256` has SHA-256
`bfb0ee334b5f81fd144d749047fb44a950fcad45b2aa7e8ac30bbeeec01f071d`.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/mesh_animation_panel/preview.rs` | `b219da79311525f5d0e4050b55dad3b092efe62b8d4a81c308b925759f9360e1` |
| `crates/orr_editor/src/mesh_animation_panel/preview/tests.rs` | `ce54f02e59768c2636e0faefdd4e6294a9dd2e6ee8bbda26ca8931d8aa98b003` |
| `crates/orr_editor/src/mesh_animation_panel/preview/transport.rs` | `9f18eb92d6938032a48bf912855bf944b968aa8c3f70713f9aaf0b00a33b16a4` |
| `crates/orr_editor/src/mesh_animation_panel/preview/transport/tests.rs` | `3a5e0642d29737ee7278ea9aad1a01a6cfe3ea07e09d5dd074f7cb9f33d19f5f` |
| `crates/orr_editor/tests/mesh_animation_candidate_ui.rs` | `cb86ebaf13f20b4ceca3e3eae7eeb75a23b5b9c9a640fbef889c12f3f34529c5` |
| `docs/editor-mesh-animation-preview.md` | `1aa9f7e4ee252855bf853ed9676690e745284299187bc642b94792fbf39367f0` |

## Limits

Rendering used llvmpipe (LLVM 19.1.7, 256 bits) software Vulkan with synthetic
egui/AccessKit input. This does not establish physical GPU, physical OS input,
native desktop-window or Windows execution acceptance. Remote CI/publication
remain separate gates. This report was added after source freezing;
`git diff --check` is clean.
