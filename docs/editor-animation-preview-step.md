# Inspect an animation in fixed source-time steps

The optional `animated-models` [Arena authoring pane](editor-animated-models.md)
now offers **Previous preview sample (1/60 s)** and **Next preview sample
(1/60 s)** beside its existing preview transport. Load an installed asset, assign
its clip to an Arena entity, and select that entity. No new package, runtime
capability or simulation clock is enabled.

Each click moves the first selected entity's transient player by approximately
1/60 second in **source clip time**, independent of binding playback speed or UI
frame rate, and leaves it not playing. From Stop it selects the configured clip
rather than sampling the rest pose. Once mode holds its terminal sample; Loop
mode uses existing forward wrapping. A previous request below zero stops at
zero—it does not wrap backwards. Zero-duration clips stay at zero.

**Preview sample time** displays the actual stored `f32` time using a
round-trippable decimal string. It is not an exact rational timestamp, authored
key index, source-video frame number or simulation tick. Repeated floating-point
steps can accumulate rounding. If a very large timestamp cannot represent the
requested small movement, the step fails instead of silently claiming progress.
Expected endpoint/zero-duration no-ops are allowed.

Time calculation and player operations happen on a temporary player; only a
successful result replaces the previous one. Invalid requests retain the
previous pose and state. Existing local-scene, Edit-mode, selected-binding and
loaded-model admission still apply. Changing selection affects which independent
player is controlled; replacing its asset retains existing preview reset rules.

These controls do not create a clip, edit asset data, change scene/binding
history, save a file or persist preview time. Play, Pause, Stop and the slider
remain available. The fixed interval is useful for examining contact poses and
short transitions but does not guarantee landing on every authored key or the
Loop clip's exact terminal key.

Focused acceptance uses real synthetic egui buttons, an installed model and the
production offscreen preview, with independent sample/pose checks and unchanged
host/history/files. Actual software-adapter receipts are distinct from physical
input, native-window and hardware-GPU coverage. Pose-to-static-prop publication
remains a separate unresolved provenance/consumer-admission design.
