# Blockout material colors

Open a local Yard3D or Room model project and **Box blockout**. Create or open
a recipe, choose **Material slot 0** or **Material slot 1** (the former white
and blue selectors), enter **Material red/green/blue** values from 0 to 1, then
click **Apply material color**. Color multiplies the existing checker on every
face using that slot. **Apply face material** assigns the chosen slot to a face.

The planar UV preview uses the applied recipe palette, not uncommitted RGB
fields. Color edits share the existing bounded Undo/Redo history. **Save box
recipe**, **Bake immutable box**, **Load model asset**, **Assign static model**
and **Save model bindings** carry the colors into the game. An assigned immutable
revision never changes automatically when a new recipe revision is baked.

Unchanged version 1 recipes retain their original exact white/blue defaults and
omit the new field. A changed color creates version 2 with a required
`material_colors` array of two RGB triples. Components must be finite and within
0 to 1; null, missing palettes for v2, palettes on v1 and malformed/duplicate
fields are rejected. A no-op color edit does not upgrade v1. Older editors cannot
open v2 authoring recipes. Runtime output remains the existing opaque static
model version 1 with two materials, the same checker and sampler; alpha stays 1.

Bake retains exact saved recipe bytes and provenance, existing no-overwrite
package publication and explicit assignment. Failure leaves document/history and
prior immutable models intact. Focused acceptance covers strict colors and v1
compatibility, history/save/reopen, actual CPU widgets and material data through
assignment plus source-hidden Room export/runtime validation. Native window/GPU
appearance and Windows publication are separate, unverified coverage.
