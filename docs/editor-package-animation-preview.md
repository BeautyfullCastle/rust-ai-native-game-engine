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

This slice deliberately has no live playback. A displayed pose stays fixed as UI
time passes. Drag the image to orbit, scroll to zoom, and **Reset preview view**
to restore the fitted camera while retaining the sampled pose. **Close animated
preview**, project changes, or selecting a replacement asset clear the preview.
Opening animation again starts at clip 0/time 0. Static and animated inspection
share one bounded GPU resource; static preview remains available separately.

Sampling does not assign a model, select an entity, save a file, change the main
editor camera, or modify Frame, history, package locks or model bindings. To assign,
separately select an entity and choose **Use animated model**, then **Save model
bindings**. That existing assignment workflow starts at clip 0 with Loop playback;
previewing another clip does not silently change the assigned animation.

Package/project/digest/request pins and stale completion rejection match
[static package preview](editor-package-preview.md). Byte verification and import
run off the UI thread. Explicit pose sampling is bounded by the admitted model,
clip and vertex limits; finite skinned bounds include the sampled deformation.
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
