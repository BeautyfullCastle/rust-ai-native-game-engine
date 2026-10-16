# Optional terrain view lab

`orr_terrain_view` converts the core's canonical `[a,c,d] / [a,d,b]` triangles
into `orr_model::{StaticModel, ModelSource, Primitive, Vertex}`. It has no GPU
dependency unless its `gpu` feature is enabled. No simulation crate depends on
this crate. The mesh omits hole cells and computes flat face normals. An all-hole
terrain without markers returns `None`, never a synthetic filled triangle.

The view requires each source coordinate to be exactly representable as `f32`
and within the existing model's +/-1,000,000 coordinate bound. Unsupported large
or sub-precision coordinates fail explicitly. Core fixed-point loads and queries
retain their full range. Texture UVs use local cell coordinates. The fixed-point
query result is retained in each marker; only its display mesh uses floats.

## Run the actual pipeline

```sh
cargo run --release --locked --offline -p orr_terrain_view --features gpu --bin terrain_lab -- /tmp/terrain-lab-new
ORR_REQUIRE_GPU=1 ORR_TERRAIN_SCREENSHOT=/tmp/terrain-acceptance.png \
  cargo test --release --locked --offline -p orr_terrain_view --features gpu -- --nocapture
```

The output directory must not exist. The binary creates a synthetic 9x9 terrain,
cooks/installs version 1.0.0, edits a peak and a hole, checks undo/redo, cooks and
installs version 1.0.1, reopens the package project, then uses only verified
`Project::read_asset` bytes for `Terrain::load`, queries and rendering. It writes
four PNGs, a JSON evidence report, the two source packages and the real installed
content-addressed package project. Runtime capability `terrain_v1` is supplied by
compiled lab code, not trusted from package content.

The required-GPU test proves actual readback pixels for terrain silhouette,
raised peak, hole absence, neighboring filled cells, moved camera, resized target,
reversible resize, and a query marker anchored at the exact sampled height. It
also checks that post-install source edits cannot alter rendered/query state.
`ORR_REQUIRE_GPU=1` fails on missing adapters. Software Vulkan is acceptable and
its adapter identity is printed. The optional screenshot is PNG. CPU tests cover
canonical topology/normals, empty terrain, package source isolation, capability
rejection and installed-content tampering.

## Deliberate limitations

- Reuses `orr_render::ModelRenderer` unchanged; no renderer API modifications
- Every draw clears its own color and fresh depth pass; it is not composited into
  `Renderer3D`'s depth or suitable as an overlay call after another scene pass
- Terrain edits and marker changes require a new immutable model and GPU upload;
  camera/light changes reuse the renderer, viewport changes recreate its depth
- Markers are part of the terrain model so they depth-test in the same pass
- Opaque diffuse/checker material only; no splat layers, LOD, shadows or streaming
- Offscreen lab, no editor/scene format/physics integration and no windowed UI
  claimed; this environment has no DISPLAY or WAYLAND_DISPLAY
