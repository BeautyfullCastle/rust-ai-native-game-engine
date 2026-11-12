# Find an invalid material tint

Material-slot authoring checks every used slot, including slots not currently
selected. A tint error now identifies its **material slot index**, **RGBA channel**
and whether the text is not a number, is nonfinite, or is outside `0..=1`.
When another slot contains the first error, **Show invalid material slot** selects
that slot so it can be repaired. Errors are ordered by slot index, then R/G/B/A.

This control changes only the panel selection. It does not stage overrides,
apply materials, save files or change editor history. Invalid tint fields continue
to disable **Stage material overrides**. After repair, stage the valid sparse
overrides and use the existing parent Apply/Save workflow explicitly.

Numeric parsing and the existing finite `0..=1` boundary are unchanged. No material
schema, renderer or model data changes are introduced. The control does not
repair corrupt or replaced admitted models; those still require reloading.
