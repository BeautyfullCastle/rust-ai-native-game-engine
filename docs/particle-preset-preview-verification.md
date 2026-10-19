# Embedded particle preset preview verification

The `particle-preset-preview` feature is an optional authoring-only extension to
`orr_particle_preset`. The original `particle-preset-editor` workflow and its
same-file production-runtime acceptance remain separate mandatory regressions.
No existing CI gate is replaced by the additive preview workflow.

## Reproduction and scope

The `particle-preset-preview` workflow installs an explicit Mesa software Vulkan
adapter, records exact test inventories, executes CPU cases individually, and
runs every selected ignored GPU case explicitly. Each executed case must report
exactly one pass, no failures, and no ignored tests; an empty filter fails.
Failed commands and their logs remain in the always-uploaded evidence artifact.

Default editor and sample checks, authoring without preview, preview without
sample runtime features, and scoped strict Clippy checks preserve feature
independence. The dependency lockfile and source cohort are checked across the
acceptance run. Existing particle/showcase/material acceptance remains in the
original authoring workflow.

CPU playback checks cover deterministic snapshot/seed/delta replay, restart,
pause, focus and visibility freezing, invalid delta rejection, a bounded
0.25-second step without accumulated debt, natural expiry, immutable accepted
snapshots and explicit close. UI acceptance must exercise production widgets
and distinguish pending drafts, newly accepted documents and captured previews.

GPU acceptance uses the production particle renderer and reads its actual
preview target; full-window screenshots separately check egui composition.
These are software-GPU rendering and synthetic widget-input checks. They do not
establish physical keyboard interaction, native OS-window behavior, hardware-GPU
performance, or frame pacing.

The saved-file regression keeps the original contract: production authoring
widgets save a new preset, and the production sample runtime consumes those exact
bytes. The input digest and unchanged bytes are checked after rendering. A preview
snapshot is not substituted for this file and is not runtime simulation state.

## Local verification record

The corrected local run passed 25 CPU test executions covering 21 distinct cases.
Eight CPU cases are new: seven pure playback contracts and one production-widget
snapshot/clock contract. Four existing authoring-state tests execute in both UI
harnesses; those repeated executions are not additional distinct coverage.

Exact inventories and executed cases:

- Shared preset schema: six CPU cases
- Production preset runtime: two CPU cases and one explicit GPU case
- Original authoring UI: five CPU cases and one explicit GPU case
- Embedded preview UI: twelve CPU cases (eight new, four shared) and one explicit
  GPU case

All three GPU cases executed individually with exactly one pass and zero ignored
tests. Only the embedded-preview GPU case is new. The original authoring UI saved
the exact file consumed by the production runtime; its bytes remained unchanged.
The new case retained six actual preview-target images and a full-window capture,
showing the accepted burst, later frame, changed accepted color, exact restart,
and exact empty expiry baseline. It also checked same-age draft exclusion,
explicit stale-snapshot refresh, pause, and native-texture cleanup on close/drop.

Ten feature-boundary and scoped strict-lint commands passed. The preview feature
tree enabled only `default,particle-preset` on the sample and `default,particles`
on the renderer, without the demo, material or imported-scene runtime features.
All 1,348 hashed source/configuration files, including the dependency lockfile,
remained unchanged across the corrected run.

The first local run passed its CPU cases but strict Clippy rejected an unused
import and four constant-size chunk iterations in the new test. Those original
logs were retained; the import and iteration style were corrected without
changing assertions or suppressing the warnings. The older authoring harness
was wired through the same constructor as the native entry to avoid unused
constructor warnings while preserving its acceptance behavior.

Prior particle/showcase/material results are separate existing coverage. Their
846-file non-editor source/configuration cohort, including the lockfile, was
verified unchanged; the five material GPU cases were not rerun for this local
preview-only slice. The original CI workflow still executes its full regression
set. Remote CI remains a separate merge gate.
