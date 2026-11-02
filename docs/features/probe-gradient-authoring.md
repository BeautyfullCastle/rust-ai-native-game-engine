# Authored probe gradient

The optional `irradiance-probes` authoring panel can fill a local Yard3D probe
grid with a linear RGB gradient along X, Y or Z. This is a spatial artistic
lighting tool, not a physical bake, directional sky model, or new rendering mode.

Open/create irradiance, choose **Gradient axis**, enter **Gradient start linear
RGB** and **Gradient end linear RGB**, then choose **Apply authored probe
gradient**. This explicitly replaces **all** existing node coefficients. The
minimum-axis plane receives the start color and the maximum plane receives the
end color; interior planes interpolate linearly. Transverse coordinates share
identical color. Colors are direction-independent irradiance, not sRGB values;
existing constant-irradiance finite/nonnegative/range admission is reused without
clamping. The current grid's applied geometry, not unsaved geometry fields, is used.

Application is one existing sidecar history transaction. **Undo irradiance** and
**Redo irradiance** restore the entire change, and **Save irradiance** remains
explicit and separate from scene Save. Invalid input leaves the document/history
unchanged. Existing matching-scene/Edit-mode restrictions remain in force.
Provenance becomes **Authored**, removing a prior bake receipt through the existing
binding API; Undo can restore its previous document. Pending asynchronous bakes
remain bound to the old revision and cannot overwrite the changed grid silently.

The feature preserves dimensions, origin, spacing, enable state, renderer/schema,
simulation and scene history. Existing 2..4 nodes per axis and64-node maximum
remain. No package data is overwritten. It is not a new general light-probe type.

Focused acceptance covers axes/dimensions/endpoints, transverse consistency,
finite/range failures and equal-endpoint constant behavior; real editor tests
cover explicit Apply/Undo/Redo/Save/reopen and stale/Play rejection. CPU and egui
checks do not claim physical GPU or native-window acceptance.
