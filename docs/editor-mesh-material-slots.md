# Edit imported mesh material slots

The Mesh agent art inspector can author a small set of per-material appearance overrides for an imported static model. These settings affect the editor's derived presentation model; they do not rewrite the cooked model, GLB, license, or package provenance.

## Choose a material slot

Load the static model through the existing Mesh agent art import flow. The material controls list material indices used by the model's primitives. Select a value in **Material slot index** to edit that slot. Indices are stable identifiers within the admitted model snapshot, not names supplied by the source package.

Each used slot has these controls:

- **Material slot tint R/G/B/A**: linear per-channel multipliers from `0` through `1`. These multiply the source material's base color and compose with the existing global agent-art tint.
- **Material slot linear filtering**: selects the existing sampler filtering mode for the slot.
- **Material slot wrap S** and **Material slot wrap T**: choose the existing `Clamp`, `Repeat`, or `Mirror` sampler mode for each UV axis.

These controls do not edit texture images, texture coordinates, UV transforms, or metallic/roughness values. A model with no used material slots has no per-slot controls.

## Apply and save

1. Edit the selected slot and choose **Stage material overrides**. Staging only prepares the material settings in the art form; it does not change the scene or the currently applied art.
2. Choose **Apply agent art** to apply the staged presentation change. Use **Undo agent art** and **Redo agent art** to review the change through the art panel's local history.
3. Save the mesh scene first, then choose **Save agent art**. This writes the existing scene-bound `.mesh-art.json` sidecar. A fresh open can restore the imported model and its overrides without the original package directory.
4. If the admitted model changes or the sidecar is malformed, stale, or conflicts with another edit, the editor rejects the settings rather than silently retargeting slot indices or changing the source model.

Resetting a slot to its source tint and sampler settings removes that slot from the sparse override data. If all slots return to their source settings, no `material_slots` override is needed in the sidecar.

## Limits and compatibility

The sidecar admits overrides only for material indices referenced by primitives in the embedded model. Duplicate, unused, or out-of-range indices are rejected. RGBA values must be finite and within `0..=1`; there may be at most 256 slot records, and derived render-model work is limited to 8 MiB. Invalid or excessive data is rejected before applying changes.

Per-slot settings are presentation metadata, not simulation or physics state. Original model and package hashes remain unchanged. Older sidecars without per-slot settings continue to use their existing appearance. Bundles whose art or prop sidecar contains the `material_slots` field require a player that advertises `ORR_MESH_MATERIAL_SLOTS_V1` in its capabilities and carries that marker in the dedicated `.orrmsl` PE section. This requirement applies even when the field is an empty array. An older player must refuse the bundle rather than ignore the field. Export keeps the existing bundle file inventory and manifest format.

This guide documents the intended authoring contract; it is not a claim that the complete UI, source-free reopen, or standalone-player acceptance has passed. Use the matching versioned run receipts for validation results.
