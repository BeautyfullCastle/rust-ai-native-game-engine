# Mesh surface editing

The Mesh agent art and Mesh props panels can apply an optional, scene-local UV and pivot transform to imported static models. Enable **Edit art UV and pivot** or **Edit prop UV and pivot** before changing the corresponding controls. The material index must be used by a primitive in that model; the panel lists the available indices. UV offset, UV scale, quarter turns, and pivot are validated before Apply or Update can change the private presentation history.

UV edits derive render geometry from the embedded model snapshot. They do not rewrite the imported package or its source GLB. Pivot edits change the embedded model's local primitive translation while preserving the original model and package provenance. Art and prop changes remain in their existing sidecars and are independent of the authoritative mesh scene; Save agent art and Save props persist them explicitly. Undo/Redo affects the corresponding local presentation history.

The controls do not change physics, navigation topology, the scene schema, or the imported package. The optional hardware-Vulkan UI acceptance test checks actual editor rendering, sidecar persistence, and source-free reopen; it is opt-in and does not claim physical OS input or validate native game behavior.

The feature uses the existing `mesh-navigation` and optional `mesh-export` features; it is not an animation workflow. Surface-derived geometry has an 8 MiB work budget. This is a bounded derived-work limit, not a process-memory cap. Legacy sidecars that omit `surface` remain compatible; an explicit `surface: null` is rejected. Export of non-identity surface edits requires the dedicated `ORR_MESH_SURFACE_V1` player capability marker.
