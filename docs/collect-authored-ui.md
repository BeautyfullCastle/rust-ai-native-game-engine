# Bounded CollectDodge authored UI

Work in progress on #94/#106. Optional `orr_sample/collect-ui` and
`orr_editor/collect-ui` consume a presentation-only UI document. Simulation
crates retain their current dependencies and checksums. `game-ui` alone does
not grant Collect UI admission. Existing `arena-korean-v1` remains unchanged.

A Collect project may declare `entry.ui` with `profile: collect-authored-v1`,
`document: ui.json`, and `font: {package, asset}`. The document is relative to
its project root, distinct from scene/sprites, regular and at most 64 KiB.
The font must be listed in the verified active package lock. Full package
contents and notices travel with exports; fonts retain their own license.

Document schema 1 contains at most 32 ordered nodes with unique ASCII IDs.
Parent containers precede children; depth is at most four. Nodes select
Title, Playing, Menu or Terminal screens. Anchors use thousandths of parent
extent, with signed logical-pixel offsets and bounded positive size. Parent
and viewport rectangles clip painting and hit testing. Later buttons own
pointer overlap. Labels bind only Score, Phase or Best; buttons allow only
Play, Menu, Continue, Restart or Quit. There are no evaluated expressions,
scripts, downloads or arbitrary actions.

The editor property panel edits the same document consumed by the runtime.
Candidate validation includes all document constraints and actual font glyph
coverage. Undo retains at most 32 snapshots. Save replaces only the UI file
atomically after checking its pinned directory and current saved contents;
it is not a transaction with scene or sprite sidecars. Editor Play keeps its
existing lifecycle; this slice does not overlay game widgets in the editor
viewport.

Menu and title block local controls while simulation keeps ticking. Play
starts a fresh authored run via the existing acknowledged restart input.
Restart edges await authoritative simulation acknowledgment, including zero
simulation-tick render updates. Completed high scores still come from the
host accumulator, never the presentation snapshot. Headless/capture does
not discover or persist user progress. Best is unavailable there.

Font validation covers every authored character plus fixed numeric/status
bindings; missing glyphs fail admission instead of silently showing tofu.
IME composition, arbitrary localization, gamepad UI navigation, and a general
visual designer remain outside this bounded slice. Arbitrary document actions
may be omitted; users author navigable screens, and Escape remains a quit path.

References consulted 2026-10-08 KST:
- https://docs.godotengine.org/en/stable/tutorials/ui/size_and_anchors.html
- https://docs.rs/egui/0.36.2/egui/struct.Ui.html

Required completion evidence: schema/font/capability negatives, real widget
clicks and clipping/DPI/resize interruption, fresh press after capture,
restart/progress retention, editor edit/save/undo/reopen, source-hidden read-only
relocated export, independent source review and exact-head required CI.

Focused commands (these must be run; listing them is not a passing result):

```sh
cargo test -p orr_sample --release --features collect-ui,collect-progress,collect-sprites,project-export --lib
cargo test -p orr_editor --release --features collect-ui --lib collect_ui_panel
cargo clippy -p orr_sample -p orr_editor --release --all-targets --features orr_sample/collect-ui,orr_sample/collect-progress,orr_sample/collect-sprites,orr_sample/project-export,orr_editor/collect-ui -- -D warnings
# Requires separately built, verified runtime/exporter paths and real GPU.
cargo test -p orr_sample --release --features collect-ui,collect-progress,project-export --test collect_project_export collect_authored_ui_export_gpu_workflow -- --ignored --exact --nocapture
```

The existing generator still produces its original no-UI template. Enabling
`collect-ui` does not silently change generated project bytes, package selection,
progress identity or existing template version. A UI-enabled saved project must
explicitly declare and package its document/font. Generator UI onboarding is a
separate remaining convenience step, not claimed by this implementation.
