# Optional Arena input-map authoring

Status: selected-action draft restoration is implemented with partial Windows
CPU widget validation. Save As is blocked by OS error 5 in this environment;
the exact executed scope is recorded below. This remains a bounded part of
#106/#94, not completed general input-editor or runtime-settings acceptance.

## Workflow

Build the standalone `orr_input_map` binary in `orr_editor` with the explicit
`input-map-editor` feature. It opens a separate authoring window, not EditorApp.
Select an action, edit its keyboard/mouse binding list, then **Apply action**.
Rejected conflicts keep the accepted map and Undo/Redo history unchanged.
Changing the selected action, Open, Undo, Redo or Defaults discards pending
unapplied widget edits. Save As saves only the accepted complete map.

**Restore action defaults** fills only the selected action's draft with its
Arena defaults, matched by action name even when an opened map reorders actions.
Inspect the draft and use **Apply action** to commit one undoable change. Other
actions, accepted history and files remain unchanged until Apply. If another
action uses a restored key, Apply reports the existing binding conflict without
stealing that key. Applying identical bindings is a no-op that preserves Redo.
**Use Arena defaults** retains its separate whole-map reset behavior.

**Discard action draft** restores the selected action's current accepted
bindings, including custom bindings. It keeps the selected action, accepted map
and Undo/Redo unchanged, clears the draft diagnostic, and does not Apply or Save.
The button is disabled whenever the draft already matches the accepted bindings,
including a defaults draft that introduces no actual change.

The fixed Arena actions are `left`, `right`, `up`, `down`, `fire`, `pause`, `quit`.
Zero through eight bindings per action are allowed, following existing Arena
semantics. Keys are the existing A–Z/arrows/Space/Escape/Enter/Tab identities;
mouse buttons are 0–7. Duplicate physical bindings across or within actions
are rejected rather than silently stolen. No gamepad backend is offered.

**Save As new file** never overwrites an existing destination, including a
symlink. **Open map (undoable)** admits the complete existing format before one
history change. History is bounded to 32 accepted changes; a fresh accepted edit
clears Redo, while invalid edits preserve it. Explicit opening of an identical
map is not a history entry. Files are neither autosaved nor monitored.

Load the resulting file with an Arena binary compiled with `input-actions`:

```
arena --input-bindings /absolute/path/to/authored.json
```

For a windowless check of the actual CLI admission path:

```
arena --input-bindings /absolute/path/to/authored.json --save-input-bindings /new/path/roundtrip.json
```

The roundtrip destination must be new. The tool displays guidance and does not
launch a process. Existing authored-project `--headless` mode rejects input-map
options; this tool does not change that contract or claim that mode tests real
keyboard input. Runtime menus still block local controls rather than stopping
network/simulation time. Arena uses latest-held controls, not acknowledged
short-tap commands. Explicit runtime maps retain their existing session-only
behavior, independent of persistent player preferences.

## Boundary and unchanged contracts

`input-map-editor` is default-off and enables the already-existing `orr_input`,
`orr_sample/input-actions` and fixture `tempfile` dependencies. The required-feature
binary and its test target are the only consumers of `input_map_authoring.rs`;
its default binary test harness remains enabled to preserve the workspace guard
against untested local modules; no app.rs/lib.rs/HUD/blockout/prefab wiring is
modified. Existing editor eframe
and egui are reused; no third-party package/version is added.

- `orr_input::ActionMap`: version1, strict known fields, at most64 actions,
  eight bindings each, bounded64KiB load/save, unique physical bindings.
- `orr_sample::arena_input::validate_map`: exactly the seven supported names,
  no gamepad, shared by authoring and the actual Arena consumer.
- `ActionMap::load_file` rejects nonregular files and direct symlinks before
  opening. This inherits its documented local filesystem boundary; it does not
  add race-safe hostile-ancestor traversal or pretend to do so.
- `ActionMap::save_new` validates first, stages in the same directory, syncs and
  publishes by non-overwriting hard link. Filesystems without hard links fail
  closed. Its existing cleanup-after-publication error remains an error; the
  tool never retries automatically or claims such an error means no file exists.
- `ArenaControls` remains the production keyboard/mouse adapter and owns
  focus/capture clearing, pause, quit and fresh-press semantics.

Simulation, Frame/replay formats, core crates, player-settings persistence,
project/package schemas, exports and security/trust policy remain unchanged.
There is no Collect/Room remapping, physical-key capture, IME, preset registry,
automatic save, external process launch or generic project export integration.

## Required acceptance before completion

1. Actual egui widgets author all seven actions and save/reopen the exact map;
   seven Undo/Redo operations recover the original and edited documents.
2. Duplicate/unknown/missing actions, bad version, gamepad, button indices,
   binding counts and oversized/malformed files preserve accepted state/history.
   Pending drafts, no-op edits, repeated Save As, symlinks and32-entry history
   limits retain the stated behavior.
3. Saved widget output is consumed by the real input-actions Arena CLI using a
   separately compiled/frozen exact executable; retain both streams and map.
4. Reloaded maps exercise all seven production control actions. Remapped
   movement must change a real Arena Frame trace and match canonical inputs at
   every sampled tick. Focus loss/capture/pause require fresh input; quit stays
   a local control event.
5. Explicit ignored GPU acceptance must execute, render/read back and retain
   default/edited PNGs, with no unavailable-adapter fallback or skipped pass.
   A no-Apply negative must fail, then a fresh restored positive must pass.
   This proves software GUI pixels, not native OS-window/physical-device use.
6. Exact inventories, selected producer features, full source/artifact fences,
   default boundary, affected strict all-target lint and independent review.
   Optional tests cannot substitute for full required exact-head CI or a fresh
   development-union review. Main/PR1 are excluded.

Tests in `tests/input_map_authoring.rs` have five ordinary contracts (four on
Windows; the symlink case is Unix-only) and two
separate mandatory ignored acceptances. Set `ORR_INPUT_MAP_ARENA_BIN` to the
absolute exact Arena executable and `ORR_INPUT_MAP_EVIDENCE` to an existing
absolute retained directory for those explicit runs. Never count ignored tests
as executed. Source-only preparation is not execution evidence.

## Selected-action restore: Windows focused result (2026-10-25)

The existing target built in release mode with unchanged profiles and offline
dependencies. Its initial run had one pass, three Save As failures with
`save bindings ... (os error 5)`, and two ignored CLI/GPU acceptances. The save
implementation groups write, sync and hard-link publication under that error;
these logs do not identify the exact failing operation. No save/security policy
was changed, no failed Save was retried, and the original failures remain.

An isolated existing CPU case then passed with added genuine egui widget input:
Open a regular reordered fixture, restore the selected action by name, preserve
accepted state/files before Apply, commit only that action, Undo/Redo, reject a
restored-key conflict and preserve Redo on an identical Apply. This fixture tests
Open, not Save As publication. The original Save/reopen assertions remain and
are not replaced by this partial pass. No test names or targets were added.
Strict Clippy for the tool binary and existing test target also exited zero.

Save As/reopen completion, ignored actual Arena CLI/GPU acceptance, native OS
input, Unix symlinks, Linux/full gate and independent final review remain open.

## Discard draft: focused Windows result (2026-10-25)

One existing memory-only egui case passed with real Discard button clicks:
conflicting, restored-default and ordinary edited drafts return to the selected
action's accepted bindings; selection, accepted map and exact Undo/Redo remain
unchanged. The button is disabled without a pending change. Existing Save-related
assertions were preserved intact in the existing Save/reopen case, which was not
run here. Test names and targets remain unchanged. Historical Save failures are
still unresolved; this pass does not replace Save, native-window/GPU or full-gate
acceptance. Strict Clippy for the selected tool binary and test target exited zero.

## References and provenance

- [#106 original action-map handoff](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/106#issuecomment-6011160152)
- [New bounded scope](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/106#issuecomment-6105580445)
- Existing source: `orr_input/src/lib.rs`, `orr_sample/src/arena_input.rs`,
  `orr_sample/src/bin/arena.rs`, `orr_sample/src/player_settings.rs` on development
  `11c47bd79aee14a25bd062f3433e2c721589c06f`.
- Official API documentation shipped in the locked crates was inspected:
  eframe0.36.2 `run_native`/`App::ui`, egui0.36.2 `ComboBox`/`TextEdit`, and
  egui_kittest0.36.2 `Harness::render`. This uses the existing API versions,
  not an assumed latest-release signature.

Official references checked during preparation: [egui0.36.2 ComboBox](https://docs.rs/egui/latest/egui/containers/struct.ComboBox.html) and [eframe0.36.2 App/run_native](https://docs.rs/eframe/latest/eframe/). The Cargo.lock pins the implementation version; a future latest-doc change does not authorize an API upgrade.

## Dedicated gates

`tools/check-input-map-authoring.sh contracts` checks the exact seven-test/full
and two-test/ignored inventories, executes each of the five ordinary contracts
as an exact singleton, verifies the enabled empty binary harness and disabled
feature inventory, runs the existing workspace binary guards and default/feature
strict checks. `acceptance` freezes an exact input-actions Arena producer, runs
both mandatory ignored CLI/GPU singletons explicitly and retains all output.
The dedicated input-map-authoring workflow triggers on every PR and pushes to
`su/engine-correctness-hardening`; existing checks are unchanged. Its resource
checks are per-command boundaries. Local execution additionally requires the
coordinator’s admitted resource budget and continuously monitored owned guard;
this script is not authorization to take the Cargo lease.
