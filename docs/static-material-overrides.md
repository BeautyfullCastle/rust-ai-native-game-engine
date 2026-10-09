# Static material factor authoring

The optional models path supports one RGB base-color factor override per static
GUID binding. Open an existing static binding, choose an actually used material
slot, edit **Override linear RGB**, then **Apply material override**. Values are
linear, finite, and between zero and one. **Reset material override** removes the
descriptor and returns to the imported factor. Model Undo/Redo and Save remain
independent of scene Undo/Save.

The override replaces the imported RGB factor; it does not multiply that factor
a second time. Texture RGB is still decoded from sRGB and multiplied once in the
existing renderer. Imported alpha, opaque behavior, UVs, sampler, geometry and
other material slots are unchanged. This follows [Khronos glTF material factor
semantics](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#material-pbrmetallicroughness-basecolorfactor).

## Storage and compatibility

Model-sidecar version 3 adds an optional `material_override` object containing
`material_slot` (u32) and `base_color_factor` (three linear RGB values). Omit the
object to reset; null, positional arrays and scalar payloads are rejected.
Versions 1 and 2 remain readable without a
version change on Open/Save, and do not accept material overrides. The first
material edit advances the version transactionally; Undo restores the prior
version. Existing animated bindings can share the v3 document, but an override
on an animated binding is explicitly rejected. Reassigning a model clears its
override. Replacing source/package content requires explicit reassignment.

The GPU-free descriptor lives in `orr_model`; read-only binding schema and
package validation live in `orr_model_bindings`; only the editor owns history
and authoring controls. The per-instance/per-primitive Object uniform carries
effective factors. StaticModel bytes, shared Arc identity and geometry/texture
cache budgets are preserved. No simulation data, RHI, shader, dependency or
cooked-model format changes are needed.

## Runtime and bake

Room's same-asset PLAYER and KEY bindings may have independent factors. Key
collection still hides the key; restarting restores the saved appearance. Export
copies the exact sidecar and verified source bytes for a read-only runtime.
Irradiance baking uses the same effective factors, including separate per-instance
bake materials. Both runtime and portable bake identities include overrides,
so an authored material change invalidates stale bake output.

Bake workers verify source and package content hashes. The existing commit and
viewport freshness checks use file length, timestamps and file identity rather
than reading all source bytes again. An external same-size edit that preserves
every observed metadata field can therefore leave an already verified bake
visible indefinitely unless later full worker validation is triggered. The
viewport does not periodically rehash these bytes. Bounded background content
revalidation or a stronger immutable-source policy is a separate follow-up;
this slice does not add per-frame asset hashing. Apply and Reset still invalidate
the bake through the authored input fingerprint.

## Verification and limits

The focused tests exercise schema rejection, used-slot admission, immutable
assets, transactional history, real egui authoring, per-instance two-material GPU
pixels, invalid-frame atomicity, Room collection/restart and source-hidden export.
The dedicated `static-material-overrides` workflow requires these feature checks
and strict lint; inherited mandatory CI is still required independently.

This is one static RGB slot per binding, not full PBR. Animated overrides,
multiple-slot lists, texture assignment, alpha editing, reimport remapping and
atomic scene-plus-sidecar project saves remain outside scope. Issue #99 stays open.
