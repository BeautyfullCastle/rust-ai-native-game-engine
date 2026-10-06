# Static textured model slice (issue #99)

This optional view-layer crate provides a **small, explicitly supported glTF 2.0
subset**, a standalone versioned cooked model, and immutable runtime validation.
`orr_render --features models` supplies the associated indexed renderer. No
simulation crate depends on these features; fixed-point gameplay is unchanged.
The existing procedural primitive renderer is unchanged.

## Supported source contract

- Standard `.gltf` + relative `.bin`/PNG dependencies, base64 data URIs, or GLB v2
  with one JSON chunk and optional BIN chunk. Unknown/duplicate chunks reject
- Explicit indexed TRIANGLES, f32 POSITION/NORMAL/TEXCOORD_0; u8/u16/u32 indices
- Interleaved vertex stride, accessor/view offsets and finite declared POSITION
  bounds. Normals must be unit length within 1%, UV magnitude at most 65536
- Scene roots, affine node matrices **or** TRS, nonuniform and mirrored transforms,
  hierarchy composition, primitive/material slots and stable source-slot IDs
- PNG images decoding to RGBA8, including expanded palettes with alpha. RGB-only
  PNGs, APNG, JPEG, other codecs and dimensions above 2048 reject
- Explicit base-color texture on UV0, base-color factor, OPAQUE, single-sided,
  metallicFactor=0 and roughnessFactor=1. **glTF's default metallicFactor=1 is
  unsupported and rejected**, not silently treated as diffuse
- Matching nearest or linear min/mag filters, no mipmaps. Absent min/mag selects
  linear. CLAMP_TO_EDGE, REPEAT (glTF default) and MIRRORED_REPEAT are supported
  with explicit per-tap textureLoad addressing in the shader. Bilinear filtering
  happens on decoded linear RGB, including wrap seams and negative UVs

Unknown schema fields reject except inert names, extras and asset generator/
copyright metadata. In particular skins, joints/weights, animations, morphs,
sparse accessors, additional vertex attributes, extensions (even optional used
ones), compressed data, alpha blending/masking, double-sided, PBR textures,
normal/emissive/occlusion maps and texture transforms reject. This is deliberately
not a replacement for a complete glTF library or metallic-roughness PBR renderer.
The initial normal `gltf 1.4.1` registry fetch was blocked by the executor's
network policy; no third-party code was vendored or downloaded around that block.

## Import, cook, load, render

```rust,ignore
let model = orr_model::import::import_path(package_snapshot_root, "models/room.gltf")?;
let cooked = model.to_bytes()?;
// Store bytes under a caller-managed asset type; this is NOT the ORAM format.
let loaded = orr_model::StaticModel::from_bytes(&cooked)?;
let mut renderer = orr_render::ModelRenderer::new(rhi, format, loaded)?;
let lighting = orr_render::Lighting { shadows: false, ..Default::default() };
renderer.draw(target_view, target_size, &camera, &lighting)?;
```

Enable `orr_model/import` for authoring; runtime `StaticModel` does not need the
parser's PNG/base64/SHA tools. `import_path` rejects absolute, traversal,
percent-encoded and scheme URIs and canonicalizes paths within the supplied
package root, including symlinks. Package snapshots should be immutable during
import. Custom `import_with_resolver` callers must enforce the same containment
and bounded reads; it never authorizes network fetches.

Logical asset IDs are supplied normalized source paths. Primitive IDs retain
node/mesh/primitive indices. SHA-256 hashes cover exact source and external
resource bytes (embedded resources are covered by the source). Identity is
stable for reimport into the same slots; digest changes track content. Reordering
source node/mesh slots is not identity-preserving. This slice does not register
an ORAM type, add a package manager capability, or change existing cook formats.

## Resource / validation contract

The source and aggregate dependency budget is 64 MiB, JSON 4 MiB, decoded images
32 MiB, 32 images, 256 materials, 1024 nodes, depth 64, 1024 rendered primitives,
250,000 expanded vertices and 750,000 expanded indices. References, accessor
ranges/strides/alignment, triangle indices, graph cycles/multiple parents,
transforms and image metadata are checked before indexed access or GPU creation.
Cooked JSON is bounded to 64 MiB and revalidated through the same immutable
`StaticModel` constructor. It is a separate `orr_static_model` version-1 format.

The renderer uploads sRGB RGBA8 through the existing `Rhi::write_texture_rgba8`
contract. It uses actual source vertex/index buffers, inverse-transpose normals
and winding reversal for mirrored nodes. It reuses Camera3D and Lighting for a
single-sample depth-tested diffuse pass, exposure and optional ACES. Non-sRGB
color targets receive shader sRGB encoding, matching the 3D renderer convention.
Texture/factor alpha is ignored per glTF OPAQUE. Shadows explicitly reject.
This pass owns its depth buffer; composition with procedural depth/shadows,
instancing, scene placement/editor authoring and full PBR remain future work.

## Acceptance / reproducibility

`tests/fixtures/generate.py` (Python standard library) generates original,
MIT OR Apache-2.0 standard glTF and GLB twins. The notched five-sided panel and
separate marker use two primitives, two material slots, two textures, interleaved
POS/NORMAL/UV0 and an offset, rotated, nonuniform child. No runtime hardcoded
geometry is substituted for this fixture.

```sh
cargo test -p orr_model --features import --release --locked --offline
ORR_REQUIRE_GPU=1 cargo test -p orr_render --features models --test models --release --locked --offline
cargo clippy -p orr_model --features import --all-targets --release --locked --offline -- -D warnings
cargo clippy -p orr_render --features models --all-targets --release --locked --offline -- -D warnings
```

GPU tests pass real GLB import through cook/reload/upload/readback, inspect UV
quadrants/notched silhouette/two materials, transformed child, depth, mirrored
winding, inverse-transpose lighting, moved camera/resize, sRGB/non-sRGB targets,
and negative-UV/corner nearest/bilinear wrap seams against a CPU linear-space
reference. `ORR_REQUIRE_GPU=1` forbids silent adapter skips. Optional
`ORR_MODEL_SCREENSHOT=/path/model.ppm` writes a PPM review artifact from readback.
CPU tests cover malformed buffers/accessors/indices, unsupported features,
containment, graph/transform rejection, PNG/APNG and cooked revalidation.

Focused CPU/GPU tests do not replace mandatory CI. Software offscreen GPU
readback is distinct from window play, Windows validation and physical-device
runs. Full issue #99, editor placement/undo/save/reopen, package registration,
shadows and PBR are not claimed complete by this slice.

## Optional skeletal path

The separate `animation` feature supports a bounded skeletal import/runtime and
`orr_render/animation` provides actual per-instance GPU skinning. See
[ANIMATION.md](ANIMATION.md) for its distinct cooked format, limits, transform and
playback semantics, installed-content viewer and incomplete #100 scope. The
static importer above continues to reject animated content.
