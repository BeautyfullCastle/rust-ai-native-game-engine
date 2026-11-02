# Copy material slot settings

The optional mesh authoring panel can reuse a slot's tint multiplier and texture
sampler settings across other used slots of the **same admitted model session**.
This avoids repeatedly entering RGBA, linear filtering, and S/T wrapping values.

In **Per-material-slot overrides**, select a slot with valid tint values, then
press **Copy material slot settings**. Select the destination slot and press
**Paste material slot settings**. The panel identifies the copied source index.
Copy takes a snapshot: later edits to the source do not change the copied value.
Paste updates only the destination's editable fields. Press **Stage material
overrides**, then use the existing parent Apply/Update and Save workflow.

No source image, texture assignment, slot ID, model bytes, baseline, staged
payload or saved data is changed by Copy/Paste. Tint is a multiplier, not a copied
final rendered colour: different source colours/textures can still look different.
The existing sparse identity-override rules and numeric validation remain in
force. A pasted change that differs from an earlier staged result must be staged
again before parent application. Other slots are not modified.

Invalid source tint disables Copy. A previously copied valid value may be pasted
into an invalid destination to repair it; existing invalid-slot diagnostics then
recompute. Reset retains its previous semantics. A failed Copy does not replace
the last valid clipboard. Loading a model clears the clipboard, including a load
that fails admission; a replaced model also clears it and requires reload.

This is panel-local memory, not the operating-system clipboard, an asset preset,
a cross-model transfer or a new material/schema feature. Existing parent editing
and stale-model guards remain unchanged. Focused acceptance exercises actual
buttons and explicit Stage, invalid-source rejection and diagnostic recovery,
slot/texture isolation, stale/reload clearing, and staged-result mismatch.
