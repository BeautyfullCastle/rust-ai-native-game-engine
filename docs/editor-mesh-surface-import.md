# Import a model as a walkable surface

The optional `mesh-navigation` editor can turn an admitted static-model package into scene-owned walkable geometry. This is a real authoring path, not the ramp fixture or a command-line example. It uses the existing static-model package admission and staged mesh history; no public API, simulation schema, runtime profile or dependency is added.

## Author the level

1. Use **File → New mesh scene** for a clean scene, or explicitly remove and save existing graph-bound presentation sidecars and remove the mesh mission before replacing its surface.
2. Expand **Static-mesh navigation → Import walkable model**. Enter the absolute **Walkable model package root**, its declared **Walkable model asset** and **Walkable model license asset**. The package must be a dependency-free `models` content package admitted by the existing static-model importer. Supported self-contained GLB and cooked static-model assets use that importer's existing subset; arbitrary glTF extensions, animation and external resources are not introduced here.
3. Review the license and enable both rights acknowledgement and staged-geometry replacement confirmation. Choose **Stage imported walkable surface**. Entering fields or validating a package never writes the package, scene or host document.
4. Inspect the staged walkable preview. **Undo staged geometry / Redo staged geometry** operate only on the local draft. Enter valid mesh agent start/goal positions on the imported surface, then choose **Place mesh agent** to commit through the normal ERP proposal/history path. Global Undo/Redo operates on the accepted document separately.
5. Save the scene or use **File → Save mesh workspace** after explicitly applying presentation edits. The navigation graph is embedded in the scene; reopening and playing the saved level does not require the source model package.
6. Optionally author a mesh mission after placing the imported surface, save, reopen and Play. Use the existing mesh-game export for a scene/manifest/player bundle when no optional presentation sidecars are applied.

This imports geometry, not its source materials, textures or a new physics collider. Agent art and props remain separate optional presentation packages. A point navigation agent is not a general character or physics controller. No license text is silently added to the runtime graph schema; users remain responsible for the redistribution rights of their source and exported content.

## Admission and failure behavior

The converter applies each admitted primitive's node transform and the existing placement transform before quantizing coordinates to fixed point. Exact fixed-point vertices are shared across primitives so adjoining triangles retain navigation portals. Mirrored triangle winding is normalized to the graph's upward convention. All source triangles must remain nondegenerate and walkable under the selected slope profile: steep faces are rejected, not silently dropped.

Limits include 4096 source vertices, 4096 source triangles, world coordinates within ±256 and the existing 8192-byte scene graph limit. Invalid coordinates, unsupported profiles, external resources, malformed packages, missing rights/replacement confirmation and oversize geometry fail closed without coordinate clamping or truncation. Source package paths and files retain the existing non-reparse, manifest, byte, hash and license checks.

The parent's existing scene/epoch/Frame, gesture, proposal, presentation-binding and staged-history guards still apply. An active mission blocks this import even if mission-preserving grid edits are enabled; mission points are not automatically rebound onto a replacement model. A rejected parent stage leaves the host and accepted graph unchanged and keeps the import form pending. Use **Reset walkable model import form** or **Discard unplaced changes** explicitly instead of assuming a failed import was applied.

## Verification boundary

The ignored Windows x64 production-App test is `mesh_surface_import_ui::imported_model_becomes_saved_playable_mesh_geometry_without_source_dependency`. It generates a self-contained adjacent-floor GLB package and drives the actual editor UI rather than loading a prebuilt navigation graph. Set `ORR_MESH_SURFACE_IMPORT_ACCEPTANCE=1`, `ORR_REQUIRE_GPU=1`, `WGPU_BACKEND=vulkan`, and a fresh absolute `ORR_MESH_SURFACE_IMPORT_CAPTURE_DIR`. `ORR_MESH_SURFACE_IMPORT_SCENE` and `ORR_MESH_SURFACE_IMPORT_EXPORT_OUTPUT` select owned scene/export paths; export also uses `ORR_MESH_EXPORT_RUNTIME`.

Its intended acceptance covers invalid package refusal, stage preview, local and authoritative Undo/Redo, mission authoring, save, source-package-hidden fresh reopen, Play/win/replay and exact three-file export. Actual run receipts distinguish completed checks from intended checks; this guide does not claim an unexecuted test or native exported-player launch passed. UI input is synthetic egui/AccessKit/pointer input, not physical operating-system input.
