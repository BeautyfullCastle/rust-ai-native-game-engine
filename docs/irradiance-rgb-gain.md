# Adjust existing irradiance color and brightness

Enable the existing optional `orr_editor/irradiance-probes` feature. In a local
Yard3D scene, open its irradiance sidecar and expand **Irradiance probes**. Set
**Gain R**, **Gain G**, **Gain B** (each finite, 0 through 16), then choose
**Apply irradiance RGB gain**. This adjusts all probes, not just the selected
node. The initial values are one, so an accidental default click changes nothing.

Unlike constant or gradient authoring, gain multiplies all nine signed SH9
coefficients per color channel. Positive gains preserve each channel's angular
and spatial pattern, subject to floating-point rounding. Zero removes that
channel. This is linear irradiance data editing, not display exposure, a new
light simulation, or a physical bake. Tone mapping means displayed pixels need
not change proportionally.

The grid dimensions, origin, spacing and enabled state are unchanged. All source
and output coefficients must satisfy the existing finite ±10000 limit. An invalid
gain or any out-of-range result rejects the whole operation without clamping or
partially modifying the document. Only presentation data changes; scene state,
authoritative simulation and ERP history are not edited.

A successful coefficient change is one sidecar Undo entry, marks the grid
**Authored**, and removes any verified bake receipt. Undo restores the original
coefficients and provenance/receipt; Redo restores the adjustment. If every
coefficient's resulting bits are unchanged, provenance, receipt, revision,
Undo/Redo and dirty state remain unchanged. This includes identity gain and
zero-valued channels that remain zero. A pending bake cannot overwrite a changed
revision through the existing bake-publication guards.

Use **Save irradiance** separately to persist the adjusted sidecar. The imported
JSON and installed package files are never modified. Reopening uses the saved
sidecar, not the original imported JSON. Existing Edit-mode, matching scene,
coherence and proposal-preview guards apply; this does not relax remote asset
or baked-data admission.

This feature does not add reflection probes, specular IBL, additional light
transport, continuous brushes, package publication or a standalone export path.
Its focused visual acceptance uses the existing Editor viewport on a reported
software offscreen backend, not a native-window or physical-GPU guarantee.
