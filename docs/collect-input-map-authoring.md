# Optional Collect input-map authoring

The default-off `collect-input-map-editor` feature adds the standalone
`orr_collect_input_map` tool. It edits the existing version-1 keyboard map used
by `collect_dodge`; it does not start a game or modify a project, export,
package, player preference, simulation format or Arena control.

```sh
cargo run --locked -p orr_editor --features collect-input-map-editor --bin orr_collect_input_map
```

The native window defaults to 720 × 650. Its production eframe app is also used
by the widget and software-GPU acceptance tests. There is no surrogate preview
UI, file dialog dependency or new third-party package.

## Edit, accept and save

1. The **Accepted map** section shows the complete map that Save As will write.
2. Select one of the six fixed actions: `left`, `right`, `up`, `down`, `restart`
   and `quit`. Edit its zero-to-eight keyboard bindings in **Action draft**.
3. Press **Apply action** to validate the entire candidate map and accept it as
   one undoable change. A key already bound within or across actions is refused;
   the prior accepted map, Undo/Redo history and rejected draft remain intact.
4. Press **Discard draft** to restore that action from the accepted map. A
   pending draft disables switching actions, Open, Undo, Redo and Defaults until
   explicitly applied or discarded. No pending draft is silently thrown away.
5. Enter a path and choose **Save As new file**. Only the accepted map is saved,
   even while an unapplied draft is visible. Saving preserves the draft and
   history; the status message explicitly identifies accepted-only saving.
6. **Open map (undoable)** validates the entire file before replacing the map.
   Undo and Redo restore exact accepted maps. At most 32 accepted changes are
   retained, and a new accepted edit clears Redo. A no-op is not a history entry.

Use A–Z, the four arrows, Space, Escape, Enter or Tab. These are physical key
identities, not typed-character or keyboard-layout capture. Mouse and gamepad
bindings are not offered or admitted. Empty actions are valid and shown as
`Unbound`. An unbound restart warns that the keyboard cannot restart the game;
an unbound quit warns to use the normal window close control. Replaced default
keys lose their old roles.

## Capture a physical key into a draft

Each existing binding row has a **Capture binding N** control. Add a binding row first
if the action is empty. Capture replaces only that row in the selected action's
draft. It does not apply the action, save a file, add an undo entry or steal a
binding from another action. Review the draft and press **Apply action** yourself.
Live conflict feedback uses the same whole-map admission validator as Apply;
conflicts remain visible in the draft without changing the accepted map.

Capture requires a fresh physical key press supported by the existing keyboard
map. Logical characters are never substituted for missing physical identity. If
the backend supplies no physical identity, use the existing dropdown. Supported
keys remain A–Z, arrows, Space, Escape, Enter and Tab; modifier chords, mouse and
gamepad bindings are unsupported. Keys already held when capture starts must be
released before a new press can be captured; repeats and activation keys cannot
silently become bindings.

**Escape cancels capture.** To explicitly bind Escape, use the dropdown. Escape
continues to release control in the main editor. Binding Escape to movement or
restart is valid for standalone Collect but rejected by the main editor; the
dropdown compatibility warning explains this distinction. Cancellation,
focus loss or a changed authoring context leaves the draft and accepted map intact.
While capture is active, modal input isolation prevents captured key events from
also activating authoring controls, text fields or gameplay shortcuts. This is a
window-local egui event flow; there are no OS-global hooks.

Save As never replaces an existing file or direct symlink. Choose a new path for
another version. Open rejects malformed, oversized, nonregular or directly
symlinked files, and rejects maps for other games. This inherits the shared
loader's local-filesystem boundary; it is not a hostile-concurrent-ancestor
filesystem sandbox. Saving validates first, stages in the destination directory,
syncs and publishes without overwrite using the existing shared implementation.
Filesystems without hard links fail closed. A publication/cleanup error remains
an error; the tool does not automatically retry or assume that no file exists.

There is no autosave, file watching, automatic project/export persistence or
runtime launch. Closing the tool discards its in-memory unsaved accepted changes
and unapplied drafts. Keep the saved session map outside an exported project if
you want it independent of project replacement.

## Load the result in Collect

```sh
cargo build --locked -p orr_sample --features collect-dodge,input-actions --bin collect_dodge
target/debug/collect_dodge --project assets/collect_dodge_project --input-bindings /absolute/path/collect-input.json
```

The map is explicitly loaded for that player session. Restarting the player
requires passing the file again. Input maps remain window-only; combining
`--input-bindings` with headless `--hold` is refused. This tool does not invent a
headless keyboard route or change the private runtime keyboard policy.

For the complete runtime policy, including aliases, restart acknowledgments,
focus, capture and quit, see [Collect input actions](collect-input-actions.md).
The [Arena authoring tool](arena-input-map-authoring.md) remains separate and
unchanged.

## Implementation boundary

- `orr_editor/collect-input-map-editor` enables only already-present `orr_input`,
  `tempfile`, `orr_sample/collect-dodge` and `orr_sample/input-actions` dependencies
- `src/collect_input_map_authoring.rs` owns accepted document, selected draft,
  admission, file operations and bounded history; its private child
  `collect_input_map_authoring/widgets.rs` owns the standalone app and widgets
- Admission delegates to the actual
  `orr_sample::collect_app::input_actions::CollectBindings::from_map` adapter and
  the shared `ActionMap` encoded-size check; the tool defines no parallel schema
- The feature-gated binary imports the module directly. The editor library and
  main app have no new integration or default UI. Its binary test harness stays
  enabled to satisfy the workspace's local-module coverage guard

## Verification and evidence scope

Commands below describe required gates, not a claim that they passed on a given
machine or revision. Record exact source, commands, inventories and results.

```sh
cargo test --locked -p orr_editor --features collect-input-map-editor --test collect_input_map_authoring_ui -- --list
cargo test --locked -p orr_editor --features collect-input-map-editor --test collect_input_map_authoring_ui
cargo check --locked -p orr_editor --features collect-input-map-editor --bin orr_collect_input_map
cargo clippy --locked -p orr_editor --features collect-input-map-editor --bin orr_collect_input_map --test collect_input_map_authoring_ui -- -D warnings
```

The canonical integration target includes 18 state tests on Unix (16 elsewhere),
four nonignored widget tests and one explicit ignored GPU acceptance: 23 total
on Unix, 21 elsewhere. The binary harness includes the state tests again; these
are duplicate test coverage and must not be counted as additional unique tests.

The four widget tests drive the actual eframe app through egui/AccessKit nodes
and pointer clicks. They author all six actions, check conflicts, explicit draft
retention/discard, accepted-only saving, exact Undo/Redo, reopen, no-overwrite,
unbound warnings and eight editable bindings. They never mutate app state as a
shortcut for editing. Saved bytes are loaded through real `CollectBindings`;
all six mappings and obsolete keys are checked. A test-only held-sample adapter
uses the public lookup API to compare remapped and default input/tick/Frame
checksum traces through movement, collect/win and restart using real
`InProc<CollectDodgeV1, PlayHost<_>>` sessions.

That adapter test does **not** call the private runtime `Keys` policy and does
**not** prove focus/capture, queued restart edges, native-window lifecycle or
physical-key injection. Those production keyboard-policy tests remain in
`orr_sample/src/collect_app/input_runtime_tests.rs`; native-window acceptance is
its own explicitly separate scope.

GPU acceptance must be invoked explicitly with an existing absolute evidence
directory and a working software Vulkan driver:

```sh
ORR_COLLECT_INPUT_MAP_EVIDENCE=/absolute/new-evidence-directory \
  cargo test --locked -p orr_editor --features collect-input-map-editor \
  --test collect_input_map_authoring_ui collect_map_widgets_have_real_gpu_pixels \
  -- --ignored --exact --nocapture
```

The test selects a CPU Vulkan adapter and fails if unavailable. It renders the
real production eframe app at 720 × 650, drives six actions and the full
save/reopen/Undo/Redo/no-overwrite sequence, refuses a conflicting draft, then
reloads the authored file for the real Bridge trace. It retains:

- `collect-input-map-default.png`, `collect-input-map-authored.png` and
  `collect-input-map-conflict.png`: full 720 × 650 window readbacks
- `collect-input-map-authored.json`: the map written by real Save As widgets
- `collect-input-map-acceptance.json`: adapter details, scope flags and sampled
  simulation trace; native-window, physical-key and private runtime-policy
  claims are explicitly false

Inspect the retained frames for readable controls and accepted/draft distinction.
A changed/nonblank frame alone is insufficient visual review. This acceptance
shows the default and single-binding edited layout, not a claim that every
possible eight-key combination is visible without scrolling. The ignored test
must actually run; listing it or passing ordinary tests cannot replace it.
Required exact-head CI, review and development-union checks remain in force.

### Physical capture acceptance

The dedicated `collect-input-capture` workflow supplements the existing
`collect-input-map-editor` and `editor-collect-controls` workflows; all remain
required. Its gate preserves the earlier state, widget, private runtime and main
editor integration requirements, and checks the compiled test inventories before
execution. The `collect_input_capture` target has 17 dedicated CPU cases plus
18 inherited document cases on Unix (16 on Windows): 35/33 total, none ignored.
The separate `collect_input_capture_ui` target has the same inherited cases and
one explicitly ignored capture GPU test: 19/17 total. Inherited cases are repeated
regression checks, not additional unique capture coverage.

```sh
python -m unittest discover -s tools -p test_collect_input_capture_gate.py
python tools/collect_input_capture_gate.py contracts --evidence /absolute/fresh/capture-evidence
WGPU_BACKEND=vulkan python tools/collect_input_capture_gate.py gpu --evidence /absolute/fresh/capture-evidence
```

The contracts stage creates a new evidence directory and explicitly executes the
ignored standalone authoring producer, which now authors its J/L/I/K/Enter/Q map
using physical capture. The existing private production `Keys` consumer must
read that exact file and attest its unchanged SHA-256 and actual file identity.
There is no substitute map or fallback route. The GPU stage requires the completed
CPU stage and rechecks that handoff before running either GPU acceptance. It
explicitly runs the capture GPU test and the existing actual main `EditorApp`
GPU victory/restart/reopen test. Listing an ignored test, counting its inherited
cases or successfully compiling it does not constitute execution evidence.

The capture GPU test retains six full 720 × 650 screenshots under
`capture-software-gpu`: initial controls, listening modal, physical-identity
unavailable guidance, live conflict, eighth-row scrolling and dropdown Escape
compatibility. `collect-input-capture-acceptance.json` records the actual software
Vulkan adapter, bounded visible widget rectangles and asserted UI behavior;
`capture-accepted-only.json` is the accepted map saved while a conflicting draft
remains. The gate rejects missing/duplicate frames, incorrect scope or adapter,
invalid dimensions or widget bounds, damaged PNGs and changed saved mappings.
Inspect the images for readability as well: rectangle and receipt assertions do
not replace visual review.

The separate main-editor GPU artifacts retain the existing full 1600 × 1000
windows, normalized scene captures and authoritative tick/checksum receipts for
mapped play, victory, restart and explicit reload. The producer/consumer handoff
and these main-editor receipts remain distinct evidence. These tests exercise
injected physical egui events and production policy, not native OS keyboard
injection, OS-global hooks or a native-window lifecycle claim. Strict lint,
default-feature and optional-feature boundaries, exact-head CI and review remain
mandatory.
