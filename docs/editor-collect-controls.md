# Saved Collect controls in the main editor

The existing `collect-ui` **and** `collect-input-map-editor` features together
add a session-only Collect controls row to the main `EditorApp`. Neither feature
alone enables it. The default editor, standalone Arena controls, Room controls,
Player, inspector, Yard and exported games retain their existing behavior.
There is no new feature, dependency, CLI flag, project/export preference, input
schema or simulation-core change.

## Load and use

1. Create a version-1 Collect map using the standalone authoring tool described
   in [Collect input-map authoring](collect-input-map-authoring.md), then Save As
   a new file. Applying an authoring draft does not load it into the main editor.
2. Open a Collect scene in the main editor. In Edit mode with control fully Off,
   enter that saved file's path in **Collect controls**, then click **Load
   controls**. The row displays the accepted bindings. **Default controls**
   explicitly restores the existing WASD/arrows, Space and Escape controls.
3. Start Play and take viewport control. Use the loaded physical keys for
   movement/restart. Replaced default keys lose their former actions.

Loading and defaults replacement are disabled during Play, Claiming and Active
control. Invalid, missing or inadmissible files show an error and preserve the
last accepted map. The shared bounded loader and Collect action adapter remain
authoritative. Escape may be Quit or unbound; movement or restart assigned to
Escape is rejected because Escape always releases editor control.

Bindings live only in the current editor scene session. An ordinary scene edit
retains them. Reopening/replacing a scene invalidates them even at the same path;
a fresh editor needs an explicit load again. No file watching, autosave or
project/export persistence is added. Loading controls never rewrites the map,
scene or project manifest.

## Keyboard and lifecycle policy

- Physical key identity wins over the event's logical key; when a physical key
  is unavailable, the logical key is the compatibility fallback
- Aliases remain independent and opposing movement cancels
- Escape always releases viewport control. Tab performs its bound action, or
  releases control when unbound. A differing logical Escape/Tab does not steal
  a physically mapped gameplay key
- Mapped Quit releases control and neutralizes pending input; it does not exit
  the editor. Release is observed before pumping queued input
- Restart uses the existing bounded queue and authoritative tick acknowledgment,
  with neutral rearm. OS repeat and held keys do not create fresh restart edges
- Focus loss, dialogs, Stop, control loss, replacement and reacquisition cancel
  stale intent. Previously held keys require release before a fresh press

The app-only translator feeds the existing editor input lease. The implementation
does not change the standalone production keyboard policy or the editor core.
Test-only visibility is limited to app state needed by the real-app harness.

## Mandatory acceptance

Run from the repository root, using a fresh absolute evidence directory:

```sh
python -m unittest discover -s tools -p test_editor_collect_controls_gate.py
python tools/editor_collect_controls_gate.py contracts --evidence /absolute/new-evidence
# With an explicitly selected llvmpipe Vulkan ICD and private XDG_RUNTIME_DIR:
WGPU_BACKEND=vulkan python tools/editor_collect_controls_gate.py gpu --evidence /absolute/new-evidence
```

The dedicated `editor-collect-controls.yml` workflow runs CPU contracts on Linux
and Windows and explicitly executes GPU acceptance on Linux. These commands are
requirements, not a claim that a particular source revision passed them.

The gate executes a strict ordered saved-file handoff:

1. Set `ORR_COLLECT_ACCEPTANCE_MAP` to a fresh absolute path and explicitly run
   `authoring_app_exports_standalone_acceptance_map` in the
   `collect_controls_runtime` integration target with `--ignored --exact`.
   This drives the real authoring app's Save As controls and creates the file.
2. Record its SHA-256, then explicitly run
   `collect_app::input_runtime_tests::controls_acceptance_tests::authoring_app_saved_bytes_drive_production_keys`
   in the sample library with `--ignored --exact`. This private production
   `Keys` consumer must read that exact file; missing environment/file and
   preexisting receipt fail. It never substitutes a default or generated map.
3. Validate the consumer receipt's exact path and producer-observed SHA-256,
   then independently hash the file again. The receipt identifies that its
   digest originated in the gate; unchanged-byte assertions and independent
   before/after hashes bind the consumer to the real producer file.

The gate pins individual CPU test names and rejects empty, skipped, duplicated,
missing and extra executions. It includes the main-app physical-key, alias,
focus/dialog/Stop/reclaim, scene-epoch, Tab/Escape, restart acknowledgment and
same-frame Quit regressions; pure routing tests; unchanged managed Collect/Room
lease tests; and the existing saved-map document suite (18 Unix / 16 Windows
cases). The runtime target also includes those document tests through its source
module: its complete inventory is 24 ordinary plus one ignored producer on Unix,
or 22 ordinary plus one ignored producer on Windows. The gate runs its six
runtime cases individually and counts the document suite only once through the
canonical authoring target. Counts must change only after checking the compiled
inventory. Default,
each single-feature and both-feature checks remain separate.

GPU acceptance is the explicit ignored
`authored_map_drives_actual_editor_gpu_victory_restart_reopen` test in
`collect_controls_ui`, with `ORR_COLLECT_CONTROLS_EVIDENCE` pointing to an
existing absolute directory. It uses the real authoring app and real main
`EditorApp`, an explicitly checked CPU Vulkan adapter, authored saved bytes,
physical egui key events and the real verified Bridge state. It retains:

- `collect-controls-authoring.png`: 720 × 650 authoring window
- `collect-controls-{mapped,victory,restart,reopened-victory}.png`: complete
  1600 × 1000 main-editor windows
- Matching `-viewport.png` crops and JSON containing bounded crop coordinates,
  phase, score, active actor count, tick, checksum and verified tick/checksum
- `collect-controls-authored.json` and `collect-controls-acceptance.json`

The gate checks exact dimensions and required files, authoritative win/restart,
verified state matching the displayed snapshot, normalized restart viewport
identity, fresh-app explicit reload and unchanged configuration bytes. The exact
normalized scene rectangle is `[230, 90, 1030, 792]` at the fixed 1600 × 1000
harness size. It excludes the viewport's top 24 pixels containing the variable
PLAY/global-tick overlay; every full-window capture retains that overlay. The
same rectangle is used for all phases. Restart must restore its exact pixels
and all four active actors; victory has two active actors and score two. A
nonblank image or an ordinary run that ignores GPU tests is not GPU coverage.
Review the retained full windows for readable controls and the actual gameplay
outcome; automated receipt checks do not replace visual inspection. Software
GPU/egui event injection does not claim a native OS window or physical keyboard.

Keep all failed logs and captures. Existing Arena authoring, Collect input,
Room and default-editor required CI remain required, as do exact-head review and
the development-branch integration checks. Main and PR1 merging remain outside
this change's scope.
