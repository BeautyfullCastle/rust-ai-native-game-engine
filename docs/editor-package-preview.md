# Preview an installed static package model

Build the optional `package-manager` editor feature and open a saved local Room
project in Edit mode. Save scene/model/camera edits first, then choose **File >
Content packages… > Verify installed packages**. Search with **Filter verified
models** when needed.

Choose **Preview static model {package}/{asset}**. A separate **Static package
model preview** window loads that installed GLB, glTF or Orrmodel as a static
asset. No selected entity is required. This is different from **Preview package
addition**, which only previews an installation change.

- Drag the preview image with the primary mouse button to orbit
- Scroll over the image to zoom
- **Reset preview view** restores the asset's fitted default camera
- **Close static preview** closes the window and releases its GPU texture/model

The preview has its own camera and one bounded offscreen target (at most
1024 × 1024 pixels). Selecting another asset replaces the previous resource.
Closing, changing projects, or starting package operations invalidates pending
requests and releases the display resource. A request pins the current project,
package, asset path, package digest and project generation; a late result cannot
replace a newer request. Loading and byte verification run off the UI thread,
with one worker and one latest waiting request.

The asset must still be present in the current verified lock and pass the normal
static model loader. Removed, changed, corrupt and stale results are refused;
errors stay in the preview window until it is closed or another asset is chosen.
Static preview does not enable animation or bypass compiled package capabilities.
Finite bounds include imported node transforms. The package store assumes trusted
local directories: checks are observations, not atomic isolation from hostile
concurrent filesystem rewrites.

Previewing does not assign a model, install content, save files, select an entity,
change the main editor camera, alter model bindings, or write simulation Frame or
history. To assign a model, separately select an entity and choose **Use model**;
then choose **Save model bindings** in the inspector. Reopening the project
re-admits the saved assignment through the existing workflow.

## Actual-UI acceptance

`package_model_preview_ui` drives the production EditorApp with software Vulkan.
Run its ignored test with a fresh absolute `ORR_PACKAGE_MODEL_PREVIEW_CAPTURE_DIR`:

```sh
cargo test -p orr_editor --features package-manager \
  --test package_model_preview_ui \
  verified_static_package_preview_is_read_only_and_assignment_still_reopens \
  -- --ignored --exact --nocapture --test-threads=1
```

The test copies the repository's owned sample package into an isolated fixture,
verifies it through the UI, previews two distinct GLBs, compares actual rendered
canvas pixels, exercises orbit/zoom/Reset/Close, refuses tampered fixture bytes,
and separately checks Use model → Save model bindings → reopen. PNG captures and
`receipt.json` record the run. CPU tests cover request identity, stale completion,
bounds, finite camera input and bounded target sizes.

With `room-animated-models`, animation-capable packages also support explicit
[indexed clip pose inspection](editor-package-animation-preview.md) before
assignment. Static and animated previews share one resource slot.
