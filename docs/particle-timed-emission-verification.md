# Timed particle emission verification

Schema 2 adds bounded rate and duration to presentation-only particle presets.
Schema 1 retains its original wire shape and burst-only behavior. The additive
`particle-timed-emission` workflow does not replace existing required CI,
particle authoring, embedded preview, or particle/showcase/material regressions.

## Reproduction and scope

The workflow pins exact inventories, executes CPU tests individually, and selects
ignored software-Vulkan tests explicitly. Every selected case must report exactly
one pass, no failures and zero ignored tests; empty or incomplete filters fail.
The always-uploaded artifact retains inventories, command output, original
failures, software-GPU captures, file digests and before/after source hashes.

The acceptance sequence retains the original schema-1 widget-authored file and
production runtime regression, and the original embedded-preview regression.
Separately, actual production widgets author and save a schema-2 file; the
unchanged production command-line loader reads those exact bytes into the real
sample host and particle renderer. The file is hashed before runtime acceptance
and must remain byte-for-byte unchanged afterward. A preview snapshot or a
handwritten fixture does not substitute for the saved file.

A zero-burst timed emitter must remain active before its first particle and
between particles, including requesting repaint while visible, focused and
playing. Pause, focus loss and hiding freeze presentation time; restart reuses
the accepted snapshot. Rendering and host checksums remain separate assertions.
The preview is isolated presentation state, not a simulation component.

Feature-boundary checks cover default builds, schema-only use, authoring-only
use, embedded preview without sample runtime features, and legacy particle and
material entry points. Strict scoped Clippy and frozen source/lockfile checks
remain mandatory. No dependency or lockfile change is part of this feature.

Software Vulkan and synthetic egui input demonstrate production renderer pixels
and widget behavior. They do not establish native OS-window input, physical
keyboard interaction, hardware-GPU performance or frame pacing. Remote CI is a
separate merge gate.

## Local verification record

The first frozen local run passed the schema, both runtime CPU suites, original
authoring CPU tests and embedded-preview CPU tests. Compilation of the new timed
UI harness then rejected two calls to an unavailable egui style API. The original
failure logs were retained; the test setup was corrected to use the installed
egui API without removing the repaint negative control or suppressing errors.
The corrected frozen run passed 52 CPU test executions covering 32 distinct
cases. Eleven distinct CPU cases are new: four schema/wire/bounds contracts, two
real-host runtime contracts, one shared authoring-history contract, three pure
playback contracts and one production-widget/file/repaint contract. Five shared
authoring-state cases run in all three UI harnesses, and ten playback cases run
in both preview-enabled harnesses; those 20 repeated executions are not new
coverage.

Exact inventories and executed cases:

- Shared preset schema: ten CPU cases
- Original production runtime: two CPU cases and one explicit GPU case
- Timed production runtime: two CPU cases and one explicit GPU case
- Original authoring UI: six CPU cases and one explicit GPU case
- Original embedded-preview UI: sixteen CPU cases and one explicit GPU case
- Timed authoring/preview UI: sixteen CPU cases and one explicit GPU case

All five GPU cases executed individually with exactly one pass and zero ignored
tests. Two GPU cases are new; the other three retain the original schema-1
widget/file/runtime and embedded-preview regressions. Both exact saved files
remained unchanged through their respective production runtime acceptance.
The timed runtime emitted visible authored red particles at its delayed, second
and final emission stages, each differing from the off baseline in 6,714 pixels.
The initial empty state, live inter-particle gap and final expiry matched the off
baseline exactly. Genuine pickup input produced matching host checksums across
off, dust, sparks and custom presentation modes. Timed UI acceptance separately
verified empty-emitter repaint liveness and a paused negative control, plus actual
preview-target pixels and full-window composition. Thirty-three PNG captures
were retained across the five GPU cases.

Eleven feature-boundary/scoped strict-Clippy commands and nine exact-count guard
self-tests passed. The preview feature tree enables only `default,particle-preset`
on the sample and `default,particles` on the renderer, without sample demo or
material features. All 1,351 hashed source/configuration files, including the
lockfile, remained unchanged across the corrected run.

The existing particle/showcase/material CI gates remain mandatory and were not
replaced. This local slice did not repeat the unrelated material-render audit.
The 80-file renderer/view/material source-and-test cohort, including the
lockfile, matches the prior accepted hashes. Existing material results are prior
coverage, not new timed-emission coverage. Remote CI remains a separate gate.
