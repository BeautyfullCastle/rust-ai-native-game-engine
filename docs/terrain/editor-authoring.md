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

Choose Entity, Vertex, Cell or Sculpt selection explicitly. Vertex height and cell hole
edits are numeric single-operation transactions. Cell selection uses the full
virtual grid, including absent hole geometry. Surface queries use exact XZ,
report height, normal and rise/run slope, and distinguish holes from outside.
The marker and renderer mesh are rebuilt from the same admitted revision.

## Discrete sculpt stamps

In Vertex mode, expand **Terrain sculpt stamp**. Pick a vertex in the viewport
or enter its X/Z indices, choose a radius of 0–32 grid steps, and enable **Show
sculpt footprint** to see the vertices that will change. Press **Apply sculpt
stamp** to make one transaction using one of these operations:

- **Raise/lower:** add an exact signed height change uniformly; negative values
  lower the terrain. An overflowing height rejects the entire stamp.
- **Flatten:** set the affected vertices to an absolute world height, useful
  for platforms and level building sites.
- **Smooth:** apply one 3×3 box mean, including the vertex itself. Every vertex
  reads the pre-stamp snapshot, so results do not depend on traversal order.
  Sums use i128 and means round toward negative infinity at one Q48.16 raw unit.

In **Flatten** mode, **Sample selected vertex height** copies the selected
vertex's admitted Q48.16 height into **Sculpt target height**. Pick that vertex
in the viewport or enter its X/Z indices first. Sampling reads terrain data,
not an unapplied edit in the **Terrain height** text field; even a vertex hidden
by a hole retains its exact height. Sampling changes only the brush setting,
not terrain bytes, query, dirty state, Undo/Redo or the host scene. It requires
writable attached terrain in coherent Edit mode and is disabled during a held
stroke. Select a destination and apply a stamp or start a Sculpt stroke to use
the sampled height. A stroke freezes that target when pressed as usual.

The footprint is an inclusive integer circle (`dx² + dz² <= radius²`), clipped
at the grid edges. Radius zero affects one vertex. Radius 32 affects at most
3,209 vertices, below the terrain core's 4,096-operation transaction cap;
smoothing reads at most nine original samples per affected vertex. Smoothing
neighborhoods clip at grid edges and may read one vertex beyond the footprint.
Hidden vertices inside and beside holes participate just as they do in numeric
vertex editing. Hole flags stay unchanged; smoothing does not fill holes or
treat them as a barrier.

Stamps use the same mesh, query, exact-display and aggregate viewport admission
as single-vertex editing. A failed stamp preserves canonical bytes, model cache,
query, dirty state and undo/redo history. One Undo restores the whole stamp;
repeating a no-op Flatten or Smooth does not consume history. Save/reopen uses
the existing `.orrt` format and asset identity, with no brush metadata added.
Package terrain remains read-only until copied to a scene file, and sculpting
requires coherent local Edit mode without a preview. Stamps and strokes do not
automatically rebake navigation or physics.

## Bounded drag sculpt strokes

In a local Yard3D scene, choose **Sculpt** selection, configure the same brush
operation and radius, then hold the primary pointer button over the viewport.
The mesh and query preview update while held; release commits the entire stroke
as one terrain Undo transaction. Entity, Vertex and Cell modes retain their
selection behavior. Secondary and middle buttons retain camera ownership and
interrupt a held stroke.

Before pressing, move over a visible editable surface to preview the cyan brush
footprint at the proposed center. Hovering changes no selected vertex, height
draft, terrain/query, Undo history or simulation state. The preview disappears
over holes, outside the viewport, behind another UI layer, or while authoring
is blocked, detached, in Play/preview, or read-only. Held strokes continue to
show their current center using the frozen brush radius. The yellow selection
ring remains the selected vertex, independent of the idle hover preview.

The operation, radius, terrain heights and pick surface are frozen at the start
of the stroke. Picking therefore cannot climb the changing preview. Integer
center-to-center traversal fills sparse pointer segments and takes one diagonal
step on exact ties. Collinear subdivisions through the same integer centers
produce the same footprint. Every covered vertex is edited once from the
starting terrain, including Smooth's original 3×3 neighborhood; overlapping
passes and repeated stationary frames do not accumulate height changes.

One stroke admits at most 256 distinct centers, 4,096 affected vertices and
65,536 brush footprint visits. Outside-grid centers, hole centers, checked
height overflow, invalid display coordinates or viewport admission failure
cancel the whole stroke. Holes may participate in neighboring brush footprints,
but the picked surface and every rasterized center must be valid surface.
These limits bound one gesture and do not change the `.orrt` format.

Escape, lost focus, an open modal, Play/preview, a changed selection mode or
scene context, and leaving the viewport surface cancel the held stroke. A
cancel restores the exact starting terrain, dirty state, model/query and
Undo/Redo history. A context replacement retains the original terrain and its
history until explicit Close/Discard; it is hidden in the new scene. Terrain
Save, query edits and other mutations are unavailable during a held stroke.
A no-op stroke preserves the original redo branch.

All supplied pointer samples are processed in order, including multiple moves
and a press/release received in one frame. An outside or hole excursion cannot
be hidden by returning to a valid center before release. A frame-wide
Escape/focus/camera interruption fences the remaining input in that frame;
starting again requires a fresh press on a later frame.

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

Falloff/erosion, multiple terrains, terrain transforms,
streaming, LOD, vegetation and automatic package publishing remain out of scope. #102 remains
open for broader terrain production and collision acceptance. Headless egui,
CPU, software GPU, native window, physical device, remote CI and publication
results must be reported separately; a successful focused run is not a full
workspace or Windows validation claim.
