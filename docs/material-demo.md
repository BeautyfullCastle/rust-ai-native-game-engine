# Playable scalar-material preview

The optional material preview dresses the existing CollectDodge pickup scene with
three authored materials: matte floor (metallic 0, roughness .75), coated blue
player (0, .25), and gold metal pickups (1, .18). The default `arena` executable
remains the existing 2D game. This preview is an incremental sample, not a complete
new default game or a full glTF PBR importer.

```sh
cargo run -p orr_sample --features material-demo --bin material_demo -- \
  --materials showcase --particles sparks
```

WASD/arrows move. Collect the gold spheres. R starts a fresh session; Escape exits.
The title reports score, material selection, particle mode and controls. Add
`--showcase` for repeated genuine-input pickup cycles. Focus loss pauses that
showcase and clears held keys. The two-second cycle creates a new game session;
there is no synthetic pickup event. No audio or learning HUD is added by this
slice.

Use `--materials legacy` for the same imported geometry, placements, camera,
lighting and particle settings with the legacy diffuse shader. Both material
selections must be explicit. `--particles off|dust|sparks` remains required.
Dust previews a preset on pickup, not a landing event. Invalid/duplicate options
are rejected before any event loop or GPU is created.

`--headless` validates the original model data and initial placement extraction,
then runs the existing CPU-only pickup loop. It does not render, benchmark or prove
window/focus interaction. Automated logic tests cover extraction, score/checksum,
particles, pickup disappearance and restart; interactive window behavior still
needs actual window testing.

## Geometry and provenance

The sample owns original procedural source recipes under MIT OR Apache-2.0, the
workspace license. It downloads no art. The versioned `RECIPE` bytes in
`particle_demo/material_scene.rs` are hashed into each model's `$source` dependency.
That digest identifies authored parameters, not cross-platform libm output or the
cooked byte hash. Existing `StaticModel` validation and cooked round trips apply.

The unit sphere uses 24 azimuth segments, 16 latitude intervals, unique poles,
362 vertices and 720 outward triangles. The plane uses 4 vertices and 2 triangles.
The floor instance uses half-extents26 at the origin, giving a52×52 world-unit floor.
The authoritative actor-center bounds are±25.4; sphere edges and a conservative
directional-shadow footprint remain on the floor without changing simulation bounds.
Player/pickup radii remain .2, with the existing fixed-point-to-view conversion.
The original scene contains one floor, one player and two pickups: three renderer
objects and four instance draws, at most 2,162 triangles before collecting.

All models use identical opaque-white 1×1 image bytes; base-color factors are
linear. Each renderer owns its own GPU image/buffers. This is not GPU texture
sharing. Renderers/models are constructed once, while bounded placement vectors
are reused. Material selection never updates the simulation or consumes its
bridge events a second time.

## Rendering limits and comparisons

The material scene camera follows the unique authoritative player without smoothing.
Its eye offset is(2.4,3.6,-1.8), vertical FOV at most40 degrees, and horizontal
coverage is capped for aspect ratios above2. Invalid target sizes are rejected
before projection math. Existing directional light and tone mapping/exposure stay
unchanged; a local copy enables the existing shared directional shadow map centered
on the player, radius6.5. This intentionally differs from the old fixed-camera
material captures; their evidence remains attributable to the prior scene. The
procedural particle preview and default Arena are unchanged.

The shared1024×1024 Depth32Float map has4MiB payload and3×3 filtered comparisons.
Its existing normal/reference bias can produce visible acne or detached edges;
this is directional shadow grounding, not contact-shadow/AO simulation. The map
covers the visible local gameplay region and moves with the player; the floor
covers the entire arena. Actual movement-edge images and native-window interaction
remain acceptance gates, not claims established by this source change. Scalar materials have approximate ambient
and no environment/specular IBL, so metal does not receive environment reflections.
There are no environment textures, normal maps, postprocessing passes or new licensed
assets. Enabling existing shadows adds the shared depth texture and one depth pass;
three static receiver passes and optional particles still use one submission. Visual inspection and matched captures must establish that highlights
and pickup visibility are useful; this document does not claim a quality win over
another engine or a completed screenshot comparison.

Performance is not measured yet. A planned comparison uses identical imported
geometry and one scalar-material branch change, with separate warm-up and measured
frames. CPU encode/submit and synchronized host wall time are distinct from GPU
timestamp duration. SwiftShader timing, if used, describes that software adapter;
it cannot establish physical-GPU speed or interactive frame pacing.
