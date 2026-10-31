# Inspect an installed animation clip before assignment

Enable the existing optional `package-manager,room-animated-models` editor
features. Open a saved local Room project in Edit mode, save its scene/model/
camera edits, and choose **File > Content packages… > Verify installed packages**.

Packages declaring `animation` offer **Preview animated model {package}/{asset}**
for declared GLB/glTF assets. A models-only package or a static Orrmodel does not
offer this action. The normal animated loader and compiled package capabilities
remain authoritative; previewing does not convert or activate content.

The separate **Animated package model preview** window initially samples clip 0
at 0 seconds. It needs no entity selection.

1. Choose an indexed, named **Preview animation clip**
2. Enter **Preview seconds** within that clip's duration
3. Choose **Sample preview pose** to display that exact sample

The **Preview clip index** field also allows an explicit index. Changing either
draft field or choosing another clip does not alter the accepted image until
Sample is clicked. The window shows the accepted clip and time. Invalid indices,
nonfinite/negative/out-of-range times and malformed drafts keep the last accepted
pose, bounds and camera. Correct the fields and sample again. A model with no
clips reports an error rather than inventing a pose.

Use **Play preview once** to audition the valid draft clip/time with a UI-only
clock. At the endpoint playback stops; Play at the endpoint begins at zero.
**Pause preview** holds the last accepted pose. **Restart preview** samples zero
on the accepted clip, keeping the current playing/paused state. Zero-duration
clips display zero and remain stopped. Draft changes or manual Sample pause
playback; they never silently modify the accepted pose. Invalid or stale samples
pause and retain that pose. There is no loop, speed control or runtime blending.
A manually sampled or paused pose stays fixed as UI time passes. Drag the image to orbit, scroll to zoom, and **Reset preview view**
to restore the fitted camera while retaining the sampled pose. **Close animated
preview**, project changes, or selecting a replacement asset clear the preview.
Opening animation again starts at clip 0/time 0. Static and animated inspection
share one bounded GPU resource; static preview remains available separately.

Sampling and auditioning do not assign a model, select an entity, save a file, change the main
editor camera, or modify Frame, history, package locks or model bindings. To assign,
separately select an entity and choose **Use animated model**, then **Save model
bindings**. That existing assignment workflow starts at clip 0 with Loop playback;
previewing another clip does not silently change the assigned animation.

Package/project/digest/request pins and stale completion rejection match
[static package preview](editor-package-preview.md). Byte verification and import
run off the UI thread. Playback samples at most once per UI frame and 30 times per second, without
catch-up loops. It pauses on lost focus, missing frames, backward/nonfinite time
or a suspension longer than one second. Each actual sample retains the existing
four metadata-pin checks; playback does not rehash packages or traverse files.
Explicit pose sampling is bounded by the admitted model, clip and vertex limits; finite skinned bounds include the sampled deformation.
The package store assumes trusted local directories and does not provide atomic
isolation from hostile concurrent filesystem rewrites.

## Actual-UI acceptance

With software Vulkan available and a new absolute
`ORR_PACKAGE_ANIMATED_PREVIEW_CAPTURE_DIR`, run:

```sh
cargo test -p orr_editor --features package-manager,room-animated-models \
  --test package_animated_preview_ui \
  verified_animated_package_preview_samples_explicit_poses_without_authoring \
  -- --ignored --exact --nocapture --test-threads=1
```

The production EditorApp test uses isolated copies of owned sample packages. It
chooses named clip 1, samples two visibly different times, verifies unsampled drafts
and invalid indices/times retain the exact accepted canvas, checks time does not
advance the pose, exercises orbit/Reset and static replacement/Close, and compares
project/source bytes, selection, Frame/history, bindings and main camera. A
separate phase verifies explicit animated assignment → Save → reopen still uses
clip 0. PNG evidence and a typed `receipt.json` record the assertions. Run the
existing `package_model_preview_ui` acceptance too for static assignment regression.

The focused `verified_package_animation_audition_plays_pauses_and_restarts_without_authoring` acceptance uses
`ORR_PACKAGE_ANIMATION_AUDITION_CAPTURE_DIR`. It checks different real canvas
poses during playback, pause pixel freeze, restart/endpoint and lifecycle/error
handling without mutating the game or project. Software-WGPU evidence does not
certify native hardware GPU behavior. Run affected transport tests and this
acceptance for audition changes; unchanged historical coverage may be reused.
