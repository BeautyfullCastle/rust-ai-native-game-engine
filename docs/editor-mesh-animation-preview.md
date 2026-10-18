# Preview an applied mesh animation in Edit mode

The optional `mesh-animated-models` editor panel can audition an applied animation without saving or entering Play. Open **Mesh agent animation**, explicitly apply the desired animation, and use its separate preview controls:

1. Enter an admitted clip index in **Animation preview clip**.
2. Enter a decimal tick from **0 through 3600** in **Animation preview tick**. Sampling uses the existing 60 Hz animation timeline and the applied playback rate, not a new wall clock.
3. Choose **Preview animation pose**. Only the viewport pose changes; the scene, simulation Frame, agent position, document history and sidecar remain unchanged.
4. Choose **Clear animation preview** to return to the authored Frame's normal animation pose.

Preview samples the selected clip independently of the applied moving/idle choice by using a temporary copy of the applied snapshot. It does not change the authored clip assignment, rate, loop option, transform, facing or idle clip. Typing preview values alone does not change the last accepted pose. An invalid tick or clip is reported without replacing that pose; values are not silently clamped.

The command requires a matching, admitted local Edit context and an applied snapshot, without pending source/form/gesture changes. Unapplied package candidates are not previewed. Normal Play always samples the authoritative Frame; transient preview is cleared on Play or source/scene/graph/epoch/applied-history changes. Preview is not serialized and does not make the workspace dirty or block saving, duplication or export. Reopening or running an exported game never inherits an editor-only preview tick or clip.

Use [Save mesh workspace](editor-mesh-workspace-save.md) to persist explicitly applied presentation edits. Preview itself needs no save. This adds no public engine API, simulation schema, runtime profile, dependency, playback clock or required package.

The composed production-App acceptance compares raw viewport poses at ticks 0 and 16 with unchanged Frame/history/files, rejects an invalid negative tick without losing the accepted pose, clears back to the authored pose, and continues through workspace save, duplicate, source-package-hidden fresh reopen, mission Play/replay and export. Local Windows receipts identify which runs actually executed; this document does not claim an unexecuted validation passed. Test input is synthetic egui/AccessKit input, not physical operating-system input.
