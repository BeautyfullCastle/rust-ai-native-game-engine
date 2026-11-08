# Author reusable static material RGB variants

In Model import, choose Cooked static model and Static mode, supply a separate .model.json source and enable static RGB authoring. Inspect the source, explicitly choose a used material index, enter three linear RGB decimal values and a new .recolor.json identity, then complete the package/license/destination fields. Import creates an immutable model.orrmodel variant. Install and assign it, Save the binding and reopen to reuse it.

The new values replace that material's linear RGB factor. They do not multiply the old factor, change image pixels or perform sRGB/CSS-hex conversion. The existing renderer multiplies the decoded texture by the new factor, so black texels stay black. Every primitive using the selected material receives the new factor; another material sharing its image remains unchanged.

RGB values must be finite f64 numbers in inclusive [0,1] before conversion to f32. Negative tiny values or values above one reject even if f32 rounding would put them in range. Positive tiny values may become zero. The recipe retains the entered f64 values for exact provenance. Alpha keeps its original bits, including signed zero; this does not add transparency to the existing opaque rendering path. Samplers, images, geometry, transforms, other material slots and unused table entries remain intact.

The complete source is validated before selection. Missing or unused material indices reject. Existing strict five-field recipe and resource/path bounds remain. Output provenance hashes the exact generated recipe and captured direct cooked input; historical provenance remains transitive and is not resolved. Changing authoring inputs revokes consent, and import checks the inspected source hash before publication. Installed originals stay immutable.

Focused CPU tests cover numerical boundaries, preservation, publication, install/binding/history/Save/reopen and actual renderer initialization color uniforms through a recording RHI. This is CPU preparation, not GPU drawing. Windows runtime, GPU pixels, native game export, physical play and video remain unverified.
