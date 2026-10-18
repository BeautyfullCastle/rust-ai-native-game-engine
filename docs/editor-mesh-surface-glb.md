# Cook a GLB as a walkable surface package

The Mesh Navigation editor can package a self-contained static GLB and a separate license file for use as walkable geometry. Cooking creates a new local package; it does not stage or apply geometry, change the scene, or add a physics collider. The package can then be selected by the existing **Import walkable model** flow.

## Cook the package

1. Open an admitted local Mesh Navigation scene in Edit mode with a current graph revision. The cook form is under **Static-mesh navigation → Cook GLB as walkable package**.
2. Enter the absolute path to **Walkable GLB source**, **Walkable model license file**, and a **New walkable model package directory** that does not already exist. Set **Walkable package name** to 1–64 lowercase ASCII letters, digits, hyphens, or underscores, and enter a bounded printable **Walkable package version**.
3. Review the rights for both files and enable **I have reviewed rights to use this walkable model**. This acknowledgement is not a legal determination. Changing either input file clears the acknowledgement.
4. Choose **Cook walkable model package**. The worker validates and copies the GLB and license bytes into a fresh static `models` package with a generated `orr.package.json`. Its files are `model.glb`, `LICENSE.txt`, and `orr.package.json`; the destination is never overwritten.

The success status reports the destination and source/license SHA-256 values, and explicitly states that geometry was not staged or applied. If the scene, graph, or inputs change while cooking, a valid package is retained but is not selected for the current editor context. A worker-disconnect status advises inspecting the destination before retrying.

## Stage and save geometry

1. A successful cook in the current scene/graph context fills the existing **Import walkable model** package root, model, and license fields. Cooking does not set the importer’s rights acknowledgement or geometry-replacement confirmation; review the license and explicitly provide both confirmations before staging. If the context changed during cooking, the package is retained but not selected.
2. Choose **Stage imported walkable surface** to inspect a candidate. Importing and staging are separate from authoritative placement; a rejected or stale candidate leaves the host graph unchanged.
3. Use **Undo staged geometry** and **Redo staged geometry** for local draft history. Place the mesh agent on the candidate surface, then choose **Place mesh agent** to commit through the normal editor transaction. Save the scene or use **File → Save mesh workspace** after applying any presentation edits.
4. Reopen the scene without the source package to verify that the walkable graph is embedded in the scene. The source GLB/package is not required for reopening or playing that saved graph.

The cook step produces the package and, when the captured editor context is still current, populates the existing importer fields. It does not apply geometry. Existing Mesh Navigation admission, context, geometry, and mission guards still apply; some geometry can cook successfully but later be rejected during staging because it exceeds walkability, fixed-point, or graph-budget limits. A mission or bound presentation may also prevent replacing the graph until explicitly resolved.

## Supported input and limits

The cooker uses the existing static-model package/import path; it is not a general glTF converter. Input must be a self-contained `.glb` accepted by that path. External resources, malformed packages, unsupported model features, missing or invalid UTF-8 license text, unacknowledged rights, and an existing destination fail closed during cooking. Walkability, fixed-point, and graph-budget checks occur during staging and can reject an already cooked package. The license is preserved as package metadata; users remain responsible for source and redistribution rights.

This workflow authors navigation geometry only. It does not import source materials or textures, create collision, or turn the point-navigation agent into a general character or physics controller. Agent art and other presentation sidecars remain separate authoring features.

## Verification

The production-App acceptance test is `mesh_surface_glb_ui::raw_glb_is_cooked_staged_saved_reopened_played_and_exported_without_sources`. It uses a generated self-contained GLB fixture and synthetic egui/AccessKit input, not physical OS input. Its opt-in Windows GPU run uses `ORR_MESH_SURFACE_GLB_ACCEPTANCE=1`, `ORR_REQUIRE_GPU=1`, `WGPU_BACKEND=vulkan`, and a fresh absolute `ORR_MESH_SURFACE_GLB_CAPTURE_DIR`; optional scene, package, and export paths are supplied with `ORR_MESH_SURFACE_GLB_SCENE`, `ORR_MESH_SURFACE_GLB_PACKAGE_OUTPUT`, and `ORR_MESH_SURFACE_GLB_EXPORT_OUTPUT`. Export uses the matching `ORR_MESH_EXPORT_RUNTIME` player.

The intended workflow checks package cooking, candidate staging without host mutation, staged and authoritative undo/redo, mission play and replay, source-free reopening, and an exact three-file scene/manifest/player export. Run-specific receipts are the evidence of completed checks; this guide alone does not claim the GLB acceptance or a native standalone-player launch passed.
