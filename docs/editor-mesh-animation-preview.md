# Preview a loaded or applied mesh animation in Edit mode

The optional `mesh-animated-models` editor panel can audition an applied animation without saving or entering Play. Open **Mesh agent animation**, select a local package, acknowledge its usage rights, and choose **Load mesh animation**. You can audition this admitted candidate before applying it, or preview the currently applied animation when no candidate is loaded:

1. Select a named, indexed clip in **Animation preview clip choice**, or enter an admitted index in **Animation preview clip**. Names are labels; indices identify clips.
2. Enter a decimal tick from **0 through 3600** in **Animation preview tick**. Manual sampling uses the existing 60 Hz animation timeline and selected snapshot playback rate.
3. Choose **Preview animation pose**. Only the viewport pose changes; the scene, simulation Frame, agent position, document history and sidecar remain unchanged.
4. Choose **Clear animation preview** to return to the authored Frame's normal animation pose.

Preview samples the selected clip independently of the applied moving/idle choice by using a temporary copy of the admitted candidate (when loaded) or applied snapshot. It does not change the authored clip assignment, rate, loop option, transform, facing or idle clip. Typing preview values alone does not change the last accepted pose. An invalid tick or clip is reported without replacing that pose; values are not silently clamped.

The command requires a matching, admitted local Edit context and an admitted snapshot, without pending source/form/gesture changes. Candidate previews use the loaded snapshot defaults; changing an unapplied form does not silently alter the preview. Clear returns to the currently applied presentation, or the normal agent when none is applied. Replacing or discarding a candidate clears its sample. Normal Play always samples the authoritative Frame; transient preview is cleared on Play or source/scene/graph/epoch/applied-history changes. Preview is not serialized and does not itself make the workspace dirty or block saving, duplication or export. A loaded, unapplied candidate still has the existing authoring blockers; choose Apply, or explicitly discard it, before Play or workspace persistence. Reopening or running an exported game never inherits an editor-only preview tick or clip.

Use [Save mesh workspace](editor-mesh-workspace-save.md) to persist explicitly applied presentation edits. Preview itself needs no save. This adds no public engine API, simulation schema, runtime profile, dependency, simulation clock or required package.

The composed production-App acceptance compares raw viewport poses at ticks 0 and 16 with unchanged Frame/history/files, rejects an invalid negative tick without losing the accepted pose, clears back to the authored pose, and continues through workspace save, duplicate, source-package-hidden fresh reopen, mission Play/replay and export. Local Windows receipts identify which runs actually executed; this document does not claim an unexecuted validation passed. Test input is synthetic egui/AccessKit input, not physical operating-system input.

The separate opt-in `mesh_animation_candidate_ui` acceptance uses the production EditorApp and synthetic egui input. It selects the named nonzero `pulse` clip before Apply, compares raw viewport pixels at ticks 0 and 16, checks Frame/history/source-file preservation, invalid input retention, Clear and candidate replacement, then Apply/Undo/Redo/Save/reopen and Play/Stop reset. Its receipt identifies the actual adapter and whether rendering was software; it does not claim hardware or physical-input acceptance. Run the ignored test with `mesh-animated-models` and a fresh absolute `ORR_ANIMATION_CANDIDATE_CAPTURE_DIR`.


## Live Edit-mode audition

After loading a rights-acknowledged candidate and choosing a clip, use **Play
animation preview**, **Pause animation preview**, and **Restart animation
preview** to audition it without Apply. **Loop animation preview** and
**Animation preview speed** (0.25×, 0.5×, 1×, 2×) control only this transient
presentation. The bounded UI clock never advances the simulation Frame or writes
the authored animation rate/loop settings.

Pause freezes the accepted pose. Focus loss and invalid preview parameters pause
audition; an invalid sample keeps the last accepted pose. Clear restores the
currently applied presentation, or the normal agent if none is applied. Normal
Play and source/context replacement retain the existing preview invalidation
rules. These controls do not save, apply, or create authoring history entries.

A separate live-audition software UI case verifies visible candidate motion,
exact paused pixels, restart, speed/loop controls, focus-loss/invalid-input
retention and unchanged Frame/history/source files, then Clear restoration over
an explicitly applied animation. The original static candidate acceptance still
covers Apply/Undo/Redo/Save/reopen separately.
