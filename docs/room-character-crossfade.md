# Room character crossfades

The optional `room-character` package supports bounded, presentation-only transitions
between Searching, Carrying and Escaped. Gameplay, inputs, frame bytes and replay
checksums are unchanged. The model binding remains the authority for asset identity
and local transform; the character document selects three distinct looping clips.

## Authoring

Schema 1 retains fixed 1x playback. Schema 2 supplies a rate for every state.
Schema 3 requires both the complete `speeds` object and integer `crossfade_ticks`
from 0 through 120. Existing schemas reject the new field. Zero gives the same
poses as immediate switching. Allowed rates remain 1/4, 1/2, 1, 2 and 4.
The editor's Room Character panel provides the duration control; Apply, Undo,
Redo and paired model/character saves preserve the existing source-ownership checks.
Generated character projects retain schema 1 until explicitly edited.

## Playback contract

A change observed on the next consecutive authoritative simulation tick starts at
weight zero, freezing the last visible pose. The incoming clip keeps its absolute
simulation-tick phase and authored rate. Subsequent ticks blend local translation
and scale linearly and rotation by normalized shortest-path spherical interpolation,
then rebuild hierarchy and skin palettes. At the duration endpoint the pose is the
exact incoming sample. Interrupting a transition freezes its current blended pose;
there is no queued transition graph.

App playback observes the initial frame and every actual step, including headless
steps. Rendering only looks up the already-observed pose and rejects a stale source,
tick or state. Repeated draws cannot advance the blend. Unknown tick gaps, rewind,
same-tick state corrections, rest/play changes, entity generations, tick rates,
documents, model identity and caller revision changes snap instead of inventing
missing history. Editor seek/start/stop/load/branch and App restart explicitly retire
history even at the same tick. Rejected sampling or asset edits retain prior history.

Positive crossfades admit a conservative five-pose peak allocation plus containers
against the existing 64 MiB decoded-model budget; legacy and zero-duration documents
have no crossfade surcharge. Existing workload and package-verification limits apply.

## Acceptance and limits

Focused tests cover strict documents, legacy identity, phase/rate endpoints,
interruptions, discontinuities, stale lookup, transactional failure, blend ownership,
hierarchy, palettes and deformation. App/editor/export checks exercise repeated draws,
source-hidden readonly export and simulation-byte parity. Software Vulkan pixel checks
are not evidence of hardware GPU or Windows/macOS/browser support. This is a fixed
three-state Room presentation feature, not a general animation graph or gameplay state
machine.
