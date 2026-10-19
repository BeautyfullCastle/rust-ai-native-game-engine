# Standalone particle preset authoring

The default-off editor authors presentation-only particle presets. It does not
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
the optional embedded preview is described below. Runtime event origin
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

## Optional embedded preview

```sh
cargo run -p orr_editor --features particle-preset-preview --bin orr_particle_preset
```

`particle-preset-preview` is default-off and enables only `particle-preset-editor`
and the production `orr_render/particles` renderer. It does not enable
`particle-demo` or imported scenes and adds no dependency or lockfile changes.

**Preview accepted preset** explicitly captures the validated accepted document
in a separate floating egui window. Dirty or invalid drafts are excluded. Applying,
opening, undoing, redoing or resetting accepted state leaves the captured preview
unchanged and marks it stale; click Preview again to refresh. Preview actions do
not save files or add authoring history. **Restart preview** clears the isolated
pool and replays its existing snapshot with the same fixed preview seed and origin.
**Pause/Resume** freezes/restarts presentation time; **Close** drops the pool and
unregisters the GPU texture. A naturally expired burst stays empty until Restart
or Preview; there is no automatic looping.

The preview uses the existing eframe device/queue and production particle renderer
in a fixed 320×240 RGBA target, with a centered orthographic camera (10 world units
high, eye at [0,0,20], Y up). Off-screen particles are clipped. This is an isolated
presentation example, not a running game or simulation. Unfocused, minimized,
occluded, collapsed and paused previews freeze. Transition frames reset their
clock, and a single frame is bounded to 0.25 seconds without catch-up debt. Only a
visible, focused, playing preview requests animation repaints.
If a GPU is unavailable, authoring stays usable and the preview displays a clear
unavailable message; it does not substitute fake particles or CPU circles.

The separate `particle_preset_preview_ui` acceptance target drives the production
widgets at explicit frame times. Its ignored software-Vulkan test uses the actual
App's native texture and reads the production 320×240 target, checks accepted
burst/color/time/pause/restart and dirty/stale behavior, compares natural expiry
to an independent exact empty target, verifies full-window sampled pixels and
checks native texture cleanup on Close and App drop. Set
`ORR_PARTICLE_PREVIEW_EVIDENCE` to an existing absolute directory; the test writes
`embedded-preview-full-window.png` and `embedded-preview-acceptance.json`.
The existing authoring → saved file → runtime acceptance remains separate and
unchanged. GPU captures establish software-rendered output, not native OS input,
hardware GPU performance or frame pacing.

## Timed emission (schema 2)

The draft-only **Emission mode** selector chooses **Burst (schema 1)** or
**Timed (schema 2)**. Timed mode exposes **Rate per second** and **Emitter duration
seconds**; particle lifetime remains separate. Apply is the only acceptance step.
Defaults remain schema 1. Opening/editing a schema-2 document retains its version.
Switching to schema 1 with nonzero timed fields fails Apply rather than dropping
them; switch back to Timed, zero both fields and choose a positive burst first.
A zero burst is valid in schema 2 when rate and duration are both positive.

The accepted summary distinguishes burst count, emission rate, emitter duration,
and particle lifetime. Dirty drafts, snapshot isolation, bounded 32-edit history,
Undo/Redo, accepted-only Save As, no-overwrite and invalid-Open atomicity apply to
both versions. Schema-1 documents keep their original wire format and behavior.

Preview stays playing while either emitters or particles remain. Empty initial
frames and gaps between short-lived particles still request animation repaints.
An emitter can expire without ever drawing (for example rate 0.5/s for one second).
Pause/focus/visibility clock freezing and explicit restart are unchanged. The
existing pool caps concurrent particles at 1024 even at rate 4096/s for 10 seconds.

The additive `particle_preset_timed_ui` test drives actual production widgets,
checks empty-frame repaint output (not only forced manual stepping), and exercises
schema/history/draft/file regressions. Its ignored
`timed_preview_widgets_have_real_gpu_pixels` acceptance requires software Vulkan,
`ORR_PARTICLE_TIMED_EVIDENCE` (existing absolute directory), and
`ORR_PARTICLE_TIMED_ACCEPTANCE_FILE` (new absolute filename). It saves a real
widget-authored schema-2 document for runtime acceptance and captures exact empty,
first-emission, live-gap and expired GPU targets plus the full editor window.
The old schema-1 UI/preview acceptance tests remain unchanged.
