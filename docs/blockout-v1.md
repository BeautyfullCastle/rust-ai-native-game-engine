# Box blockout v1: bounded editor authoring

Status: P2 initial slice of [#108](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/108), not completion of general modeling/UV tools. Optional under the existing default-off `models` feature; Room uses `room-project`. No Cargo dependency, importer, renderer, runtime schema, simulation or collision changes.

## Workflow

Open an admitted local Yard3D or Room project and its model bindings. Expand **Box blockout**, choose a contained `*.blockout.json` path, then **New box recipe**. The recipe path is relative to the admitted model project, never a remote host path or editable project text hint.

1. Select one of the six stable faces: +X, -X, +Y, -Y, +Z, -Z
2. Enter a signed **Move face delta**, then **Apply face move**. Positive moves along the selected outward normal; negative moves inward. Moving -X positively decreases the minimum X bound
3. Enter the face's UV offset, positive scale and quarter-turn, then **Apply face UV**
4. Choose **Checker white** or **Checker blue**, then **Apply face material**
5. **Undo blockout** and **Redo blockout** affect only the recipe. **Save box recipe** writes it; **Open box recipe** reloads it. New/Open reject dirty recipes until saved or explicitly discarded
6. **Bake immutable box** requires the current recipe to be saved and unchanged on disk. It installs a new immutable package and sets the existing model picker's package/asset fields
7. Explicitly **Load model asset**, **Assign static model**, then **Save model bindings**. Model binding Undo/Redo and Save remain separate from recipe history and scene history
8. Reopen the Room project, Play, and export with the existing Room exporter. The runtime consumes only the cooked static model through the existing package loader

Editing or baking a recipe never silently modifies an assigned mesh. This is presentation-only: the scene's bodies, collision and deterministic input/checksum contract are unchanged. Save the scene separately for any actual scene edits.

## Recipe and mesh contract

`orr_box_blockout` version 1 is an editor-owned JSON object. Bounds and each face/UV record must be objects, not struct-array aliases. Fields are required; duplicates, unknowns, unsupported versions, extra/missing array entries, nonfinite and overflowed numbers are errors. Six face records have the stable order listed above. Bounds must be finite, within +/-100, and have at least 0.001 extent on every axis.

Each face has four independent vertices, an outward unit axis normal and six counterclockwise indices. A box always has 24 vertices, 36 indices and six named primitives. There are exactly two material slots; both use one embedded nearest-filtered repeating 4x4 asymmetric checker, with 16 different cells. The second slot applies a blue base-color factor. The fixed-cost 2D preview displays the applied face UVs/material, not pending control values.

UVs start at corners (0,0), (1,0), (1,1), (0,1). Apply component-wise positive scale, then counterclockwise quarter-turn about the origin, then offset. Scale is 0.001..16; offsets are +/-16; quarter-turn is 0..3. Moving a face preserves its normalized planar coordinates. This is per-face planar authoring, not general unwrapping.

All edits validate a candidate before replacing state. Invalid edits retain the document and both history stacks. History holds at most 64 prior recipe states. A successful new edit clears redo; a no-op or failure does not.

## IO, immutable revisions and limits

Recipe reads are bounded to 16 KiB and reject symlinks, non-regular files, absolute paths, traversal and hidden path components. Parents must already exist. Static metadata checks prevent ordinary unsafe input; they are not a sandbox against a hostile process replacing paths concurrently. Recipe files are atomically replaced from a sibling temporary file after comparison with the last saved bytes. A newly created recipe uses non-clobbering publication. An unedited Open/Save preserves the exact original bytes, including whitespace.

Bake captures those exact saved recipe bytes. Their full SHA-256 appears as the cooked model's `$source` dependency. The cooked `asset_id` is always the packaged recipe name, `box.blockout.json`; primitive IDs preserve named faces under `box.blockout.json#node=`. Existing model bindings independently retain the cooked file's hash as their `source_hash`, preserving the existing binding contract. The package name is `blockout-` plus the first 48 recipe-hash characters; a prefix collision with different content is rejected rather than overwriting anything. Version is 1.0.0. A package includes:

- `box.blockout.json`: exact authoring source bytes, copied from the chosen working filename (which may be `authored.blockout.json` or another accepted name)
- `box.model.json`: validated existing cooked static model, including checker pixels
- `CHECKER-LICENSE.txt` and `NOTICE.txt`: procedural checker's dedication/provenance

Identical source bytes produce identical cooked/package bytes and an idempotent bake. Different source bytes produce a distinct identity except for a detected prefix collision. Each revision remains directly installed; old revisions stay loadable for binding Undo. No automatic pruning occurs. The existing maximum of 128 installed packages is enforced before installation. Export still enforces its independent aggregate 256 MiB/8192-file budgets and all Room limits; creating a small box does not guarantee an arbitrary pre-existing project is exportable.

Preparation verifies the consuming project's complete active lock. `install_checked` compares the captured lock and the preflighted union under its existing writer guard. The final callback rechecks recipe bytes. Failure or a changed lock/recipe retains the active lock; the existing installer may retain unreachable immutable staging objects. A failed save/bake does not change recipe state/history or model assignment.

Play, previews, incoherent scene identities and remote sessions disable authoring. Path/root authority is resolved only for the matching admitted local scene.

## Research and exclusions

[Unity ProBuilder's shape tools](https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/shape-tool.html) provide a reference for explicit primitive dimensions, and its [UV actions](https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/auto-uvs-actions.html) motivate separate scale/rotation/offset controls. This deliberately smaller tool uses the engine's existing mesh/material conventions, consistent with the [glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html); it does not add another importer.

Excluded: arbitrary topology, extrusion, vertex dragging, general unwrap, external image inputs, autoreimport, live viewport face picking, relink, collision generation and changes to simulation. The only shared integration points are small additions to `lib.rs` and `model_panel.rs`; concurrent material/animation work touching those files needs a fresh union review.

## Verification

`tools/check-blockout.sh contracts` runs named CPU invariants, models-only and Room feature checks, existing dependency isolation and strict Clippy. `tools/check-blockout.sh gpu` requires actual GPU readback and source-hidden, read-only Room runtime/export parity; absence of an adapter, runtime/exporter or isolation support is failure, never a skip-pass. Test names and positive result counts are checked explicitly. Early development results are not final frozen-candidate or hosted-CI evidence.
