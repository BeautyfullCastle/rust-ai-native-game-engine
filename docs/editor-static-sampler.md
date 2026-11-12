# Author reusable static material sampler defaults

In Model import, choose Cooked static model and Static mode, supply a separate local .model.json source and enable static material sampler authoring. Inspect the source and explicitly choose a used material index; names and image sharing do not identify a material slot. Choose nearest or bilinear filtering and independent S/T clamp, repeat or mirror wrapping, then a new logical .sampler.json identity and the normal package/license/destination fields. Import creates an immutable model.orrmodel variant. Install and assign it, then Save the binding and reopen to reuse its defaults.

Nearest preserves hard texel transitions; bilinear blends neighboring texels. Clamp holds texture edges, repeat tiles them, and mirror alternates tile direction. These are the existing renderer's single-level controls, without new mipmap or anisotropy modes. Texture bytes and UVs stay unchanged.

Only the selected material's three sampler fields change, along with the new identity and provenance. Every primitive using that material receives the defaults. Another material sharing the same image keeps its own settings. Geometry, transforms, normals, UVs, base-color factors, alpha, other sampler slots and unused image/material entries remain intact. Room uses the loaded material defaults; the separate Mesh instance-override path remains unchanged.

The complete source is validated before selection; an absent or unused material rejects. Existing strict seven-field recipe, input/output and file-path bounds remain. Output provenance hashes the exact generated recipe and direct cooked source; earlier provenance remains transitive through that captured source and is not resolved. Rights acknowledgement and current-source hash checks are required; changing source or authoring inputs revokes consent. Installed originals are not overwritten.

Focused CPU tests cover all filter/wrap combinations, selection, preservation, safe publication and install/binding/history/Save/reopen/Room consumption. Windows runtime, GPU pixels, native export, physical play and video remain unverified.
