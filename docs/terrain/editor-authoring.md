# Bounded Yard3D terrain authoring (#102)

Build the editor with `--features terrain`. This adds only the optional terrain
core/view crates and existing static-model support. Animation, navigation,
irradiance probes and the terrain lab GPU renderer are not prerequisites.

Open a saved local Yard3D scene in Edit mode. Expand **Terrain authoring** in the Inspector.
Create one heightfield using an asset identity, 2–129 vertices on each axis,
world XZ origin, positive spacing and initial height, or open a canonical
scene-relative `.orrt` file. Numeric fields are parsed directly as exact Q48.16
decimal text. Terrain has its own explicit Save, Undo and Redo controls; it does
not modify the ERP document, entity GUIDs, host undo history or simulation state.

Choose Entity, Vertex or Cell selection explicitly. Vertex height and cell hole
edits are numeric single-operation transactions. Cell selection uses the full
virtual grid, including absent hole geometry. Surface queries use exact XZ,
report height, normal and rise/run slope, and distinguish holes from outside.
The marker and renderer mesh are rebuilt from the same admitted revision.

A candidate edit, undo, redo or load is validated before admission. Display
coordinates must be exactly representable as f32 and inside +/-1,000,000;
unsupported fixed-point values are rejected instead of silently rounded.
All-hole terrain generates no model. Terrain and entity models share the main
viewport's imported-scene depth/shadow renderer and aggregate budgets. Terrain
has no ECS identity and never suppresses another entity's collider rendering.

Save uses a staged same-directory file replacement. Scene-relative paths reject
symlinks, traversal and installed object directories. Unsaved content blocks
replacement and scene-path changes until saved or explicitly discarded.
Installed `terrain_v1` package content is digest-verified and read-only. Copy it
to a new scene-relative terrain file before editing; installed bytes stay intact.
The Yard host advertises only capabilities compiled into its consuming loader.

## Lighting and physics boundaries

Terrain is not part of the current bounded static diffuse bake snapshot or
fingerprint. While terrain is attached, new static sun bakes and application of
saved **Baked** receipts are suspended, with an explanation in the UI. Probe
sidecars, coefficients, receipts and history remain untouched. Authored/imported
manual probes may continue. Removing terrain requires ordinary source/fingerprint
revalidation before a saved bake applies again.

Terrain is presentation and deterministic surface-query data only. Rigid-body
terrain collision is unsupported: no hidden body/boxes or fake contacts are
created. The host's sphere/capsule/box physics is unchanged. Navigation remains a
separate revision-bound consumer and is not included in this feature.

Brushes, drag strokes, multiple terrains, terrain transforms, streaming, LOD,
vegetation and automatic package publishing remain out of scope. #102 remains
open for broader terrain production and collision acceptance. Headless egui,
CPU, software GPU, native window, physical device, remote CI and publication
results must be reported separately; a successful focused run is not a full
workspace or Windows validation claim.
