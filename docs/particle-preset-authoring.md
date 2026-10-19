# Standalone particle preset authoring

The default-off editor authors presentation-only burst presets. It does not
modify a project, simulation, runtime origin/seed, particle pool, or main editor.
Existing eframe/egui dependencies are reused; no new external dependency is needed.

```sh
cargo run -p orr_editor --features particle-preset-editor --bin orr_particle_preset
```

Edit burst count, lifetime, velocity and independent +/- velocity jitter, start/end
world-space diameter, straight RGBA start/end colors, and Alpha/Additive blend.
All numeric controls retain their text, including invalid text. **Apply draft**
parses and validates the whole candidate before replacing accepted state. Failure
preserves accepted state, draft, and history. **Discard draft** explicitly restores
the accepted values. The accepted summary is separate from the editable draft.

**Undo/Redo** retains up to 32 accepted edits. Open and Defaults are also undoable;
invalid Open retains accepted state and history. A pending draft blocks Open,
Undo/Redo and Defaults until Apply or Discard. **Save As new file** writes only
accepted state, even with an invalid pending draft, and never overwrites existing
paths. Existing files must be opened or a new destination chosen explicitly.
The shared preset loader and publisher enforce the schema, bounds and filesystem
admission. See their API documentation for local-file race and publication limits.

Use the exact saved file in the production `particle_demo --preset FILE` workflow;
this standalone UI deliberately does not embed a GPU preview. Runtime event origin
and seed are supplied by the existing presentation event adapter, not the document.

Run the production preview with both explicit feature gates:

```sh
cargo run -p orr_sample --features particle-demo,particle-preset --bin particle_demo -- --preset FILE --showcase
```

## Acceptance

```sh
cargo test -p orr_editor --features particle-preset-editor --bin orr_particle_preset
cargo test -p orr_editor --features particle-preset-editor --test particle_preset_authoring_ui
```

The UI test drives production eframe text inputs and combo/buttons, edits every
exposed numeric field, changes both blends, checks atomic invalid Apply, Discard,
Undo/Redo, accepted-only Save As while dirty, no overwrite, and valid/invalid Open.
The ignored `particle_preset_widgets_have_real_gpu_pixels` test requires a software
Vulkan adapter. Set `ORR_PARTICLE_PRESET_EVIDENCE` to an existing absolute evidence
directory and `ORR_PARTICLE_PRESET_ACCEPTANCE_FILE` to a new absolute filename.
It retains actual full-window default/authored PNGs, an acceptance JSON report,
and the exact widget-authored preset. The runtime GPU acceptance consumes that
same file; it must not substitute a fixture or regenerate the document.

The eframe harness renders production GUI pixels but does not create a native OS
window or inject a physical keyboard. The preset document's deterministic JSON
round trip is separate from the runtime's presentation determinism claims.

Preset JSON uses a versioned object schema and a 64 KiB input bound. Unknown,
duplicate, missing and runtime-owned fields are rejected. Default sample/editor
checks and strict scoped Clippy checks preserve the optional feature boundary.

The additive `particle-preset-authoring` workflow pins exact test inventories and
executes every selected ignored GPU test with a one-pass, zero-ignored guard. It
binds the UI-saved input to the runtime with a digest and verifies unchanged bytes.
Existing particle/showcase/material regression assertions remain intact.
Software Vulkan captures and synthetic widget inputs do not establish native
window interaction, physical keyboard events, hardware GPU performance or frame pacing.

## Acceptance record

The first local run passed the new CPU, widget, feature-boundary, strict-lint and
same-file runtime GPU checks. An additional existing grounding regression initially
failed because its required capture-directory environment variable was missing.
That setup failure was retained; the workflow was corrected to provide a fresh
capture directory without changing the grounding test or its assertions.

The corrected local acceptance completed with 29 CPU tests, nine individually
executed software-GPU tests, and seven feature-boundary/lint commands passing.
The nine GPU cases comprise the editor, the authored-file runtime, and seven
existing particle/showcase/material regression checks; existing checks are not
claimed as new coverage. The UI-authored green preset visibly differed from both
built-ins, evolved over time, and naturally expired to the exact off baseline.
All compared genuine-input host checksums matched. The final UI capture shows the
validated preset reopened successfully. The source cohort and dependency lockfile
were unchanged throughout the corrected run. Remote CI remains a separate gate.
