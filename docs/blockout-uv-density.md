# Recipe-space box UV density

This optional `models` editor convenience computes the selected box face's UV scale from the current recipe dimensions. It adds no dependencies, recipe/runtime schema, renderer, importer, collision or simulation changes. It builds on the existing [box blockout workflow](blockout-v1.md).

## Use it

1. Open an admitted local Yard3D or Room scene and its model bindings, select a document entity, then create or open a box recipe
2. In **Box blockout**, choose the face. Existing offsets and quarter-turn values may remain unapplied in the UV controls
3. Expand **Recipe-space UV density** and enter **Model units per repeat**. For example, `2` asks for one full texture repeat every two recipe units
4. Click **Calculate UV scale**. This reports the proposed U/V scales without changing the draft, recipe, history, files or model bindings
5. Click **Stage UV scale**. Only the existing **Face U scale** and **Face V scale** draft values change; offset and quarter-turn are retained
6. Click the existing **Apply face UV**. This is the recipe edit and the single recipe-history entry. The existing planar checker preview now reflects the applied UVs; Undo/Redo blockout work as before
7. **Save box recipe**, then **Bake immutable box**, **Load model asset**, **Assign static model** and **Save model bindings** to update the main 3D viewport's assigned immutable model

Calculating, staging or applying recipe UVs does not alter an already assigned immutable model. Model binding history and scene history remain separate.

The input must be positive and finite. Both computed scales must fit the recipe's existing `0.001..16` range. An out-of-range result is rejected, never clamped. Invalid input does not replace the draft or create history.

## Coordinates and resizing

The calculation uses recipe/model coordinates, excluding model binding scale and scene transforms. It does not promise equal repeat spacing after a nonuniform binding transform.

Let X, Y and Z be the positive recipe bounds extents, and D the model units per repeat:

| Selected faces | U scale | V scale |
|---|---:|---:|
| +X, -X | Y / D | Z / D |
| +Y, -Y | X / D | Z / D |
| +Z, -Z | X / D | Y / D |

These are the baked face's planar edge directions. Scale is applied before the existing quarter-turn, then the offset. A 90° or 270° turn does not swap the calculated scales. Material choice and the other five faces are untouched.

There is no automatic density lock. After **Apply face move**, the previously applied UV scale remains. Explicitly calculate, stage and apply again if the resized face should have the same model-unit spacing. For a +X face with Y=3, Z=4 and D=2, the scale is `[1.5, 2]`; increasing Y to 4 leaves that old scale until recalculation produces `[2, 2]`.

## Stale-context and admission checks

Stage rechecks the admitted local editable scene, live selected document entity and matching recipe/project root. It compares the calculation's value snapshot with the current model sidecar path and document, scene path, selected entity, recipe path and path field, full recipe, selected face, full UV draft (both offsets, both scales and quarter-turn), and density input. If those values differ, it refuses without changing the draft and requires a fresh calculation.

This is a current-value snapshot guard, not a document-generation or filesystem epoch fence. Returning to identical current context values is safe for this scale-only draft operation; existing Apply/Save/Bake retain their own validation and IO protections. Calculation or staging never writes files or accepts remote host paths as local authority. Play, proposal previews, missing/incoherent selection, mismatched roots and unavailable local edit admission disable the controls.

## Verification

Core math and caller contracts:

```sh
cargo test -p orr_editor --features models --test blockout_uv_density
cargo test -p orr_editor --features models --lib blockout_uv_density
cargo test -p orr_editor --features models --test blockout
cargo check -p orr_editor --no-default-features
cargo check -p orr_editor --features models
```

The ignored production-editor GPU test must be invoked explicitly with an actual usable GPU backend and a new capture directory:

```sh
ORR_REQUIRE_GPU=1 ORR_UV_DENSITY_CAPTURE_DIR=/absolute/new/evidence-directory \
  cargo test -p orr_editor --features models --test blockout_uv_density_ui \
  density_widgets_preserve_draft_and_render_after_existing_bake -- \
  --ignored --exact --nocapture
```

The acceptance opens the complete EditorApp on Yard3D, changes nonuniform box dimensions and UV controls through actual widgets, uses a nonuniform model binding scale, and retains 1800×1500 full-editor PNGs showing visible controls from the outset. Fixed-camera readbacks use a remembered 560-point Inspector and 230-point hierarchy, producing a 1010×1383 viewport; the full box panel is not claimed to fit the default width. A separate full-editor capture collapses the existing box and component editors, then asserts the new density controls and staged-status text fit the default 340-point Inspector. It verifies staging has no recipe/history/file/binding effect; Apply changes only the selected UV; Undo/Redo and Save/Open restore the exact recipe; existing Bake/Load/Assign changes fixed-camera +X checker pixels while +Z pixels remain identical. A literal rotated-UV oracle checks the baked model independently of the calculation helper. It also checks explicit recalculation after resize, Play disablement, fresh reopen and unchanged scene/checksum/history.

`acceptance-receipt.json` records source/file hashes, actual pixel differences, numeric inputs and retained immutable revisions. `capture-inventory.json` inventories the evidence files. An ignored test listing, a build-only result or a failed run is not GPU acceptance evidence. Software Vulkan proves the production graphics path on that backend; it is not evidence of hardware-GPU or platform-wide acceptance.
