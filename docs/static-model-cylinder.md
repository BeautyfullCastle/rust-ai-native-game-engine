# Offline capped static-cylinder authoring

`static_model_cylinder` creates one ordinary cooked static model from a small
recipe. This private authoring example is optional under `orr_model/import`.
It adds no runtime primitive, importer, simulation component, package format,
model schema, Cargo feature or dependency.

```sh
cargo run --locked -p orr_model --features import --example static_model_cylinder -- \
  --recipe props/key.cylinder.json \
  --asset-id props/key.cylinder.json \
  --output key.model.json
```

The recipe file must already exist; the output file must not exist. Successful
publication or help exits 0. Errors exit 2. Without `import`, the example exits
2, including for `--help`. There is no install, overwrite, force, stdin or
network option. Existing package APIs can install the resulting cooked file.

## Recipe and limits

```json
{
  "format": "orr_static_model_cylinder",
  "version": 1,
  "radius": 0.5,
  "height": 1.0,
  "segments": 32
}
```

The recipe is at most 16 KiB and must be an object containing exactly these
five required fields. Duplicate, unknown and missing fields, positional array
aliases, null values and incorrect JSON types are errors. Radius and height
must be finite JSON numbers in `[0.001, 1000000]`, checked as `f64` before
conversion to `f32`. Segments must be the integer `8`, `16`, `32`, `64` or `128`.

The logical asset identity is a portable relative ASCII path ending in
`.cylinder.json`, at most 240 bytes. Hidden, traversal, empty and reserved-device
segments are refused. It names the output model and provenance; it does not
redirect the physical recipe path. The sole dependency is `$source`, whose
SHA-256 covers the exact recipe bytes, including whitespace.

Input and destination ancestry must be trusted, stable, local and link-free.
The authoring CLI rejects nonregular recipe files, symlink ancestry,
package-store destinations and nonportable output names. A checked staged file
is published without replacing an existing file. Failure leaves no partial
output and preserves input bytes. These portable filesystem checks are not a
sandbox against hostile concurrent directory replacement.

## Geometry, normals and UVs

The cylinder is centered on the origin, aligned with Y, with radius in X/Z and
ends at `y = -height/2` and `y = +height/2`. It contains exactly three primitives
in this order:

1. Side: smooth radial normals and a duplicated UV seam
2. Top: hard `+Y` normals
3. Bottom: hard `-Y` normals

Primitive identities are the recipe identity followed by
`#node=0/mesh=0/primitive=0`, `1` and `2`. Each primitive has its own default-white
material slot; the slots share a white image. The output uses the existing
validated `StaticModel` version 1 representation and identity transforms.

For `N` segments the model has `4N + 4` vertices, `12N` indices and `4N`
nondegenerate outward-facing triangles. Side UVs are `[i/N, 1]` at the bottom
and `[i/N, 0]` at the top, with equal seam positions/normals and distinct U=0/1.
Cap UVs map the circle to `[0.5 + 0.5 cos(theta), 0.5 + 0.5 sin(theta)]`.
Separate cap vertices preserve hard edges. Welding equal positions for topology
validation produces a closed, consistently oriented surface; render vertices
remain split for normals and UVs.

### Numeric contract

Ring directions come from a fixed 33-entry quarter-circle table stored as
binary32 bit patterns. Every supported segment count selects entries from a
128-step circle; quadrants are reconstructed explicitly. The tool does not
call platform trigonometric functions. Dimensions and derived geometry use
binary32 authoring arithmetic, with explicit seam reuse and stable ordering.
Repeated identical input bytes and identity under the same compiled producer
must produce identical cooked bytes. Exact recipe bytes remain significant to
provenance even when two recipes describe the same geometry.

This is presentation-only asset generation. It does not change fixed-point
simulation determinism or promise unverified cross-platform byte identity.
CPU tests use independent geometric oracles, not the production ring table,
and check supported segment counts, bounds, normals, UVs, outward winding,
closed edge pairing, provenance and deterministic repeated serialization.

## Package and Room acceptance

The additive acceptance workflow requires all four rows: CPU Linux, CPU
Windows, Linux software-Vulkan rendering, and Linux editor UI. Every row builds
fresh current-source producers in its own empty target directory and retains
executables, hashes, logs and failure evidence. All Rust test inventories and
executed result counts are exact; an ignored fixture must actually execute.
Missing adapters, skipped tests, partial captures or mismatched source/producer
hashes fail acceptance.

The GPU fixture installs actual CLI output through existing package APIs,
loads installed bytes after source hiding and renders both direct-white models
and test-only diagnostic material controls. Independent geometry/raster oracles
and wrong-geometry, UV and winding controls distinguish a visible image from a
correct cylinder. Top and bottom views test both caps.

The editor fixture uses an actual Room KEY entity and existing package UI:
preview is read-only, Use Model creates one undoable binding change, Undo/Redo
preserves identity, and Save followed by fresh source-hidden reopen preserves
the installed binding and rendered result. Collecting the KEY hides its model;
restart restores it. Render-independent gameplay controls establish that the
model binding does not alter the Room key interaction or simulation outcome.
These are acceptance obligations, not evidence that a particular local or
hosted run has already passed.

The gate also requires strict Clippy for affected targets, feature-off refusal,
complete source inventories and unchanged final source and retained producer
hashes. Python must inherit `PYTHONDONTWRITEBYTECODE=1`; optimized Python is
refused. A local focused test cannot certify the other hosted rows. Existing
acceptance workflows and checks are preserved and are explicitly **not run by
this feature gate**; their own required checks remain mandatory. A sandbox
restriction or unavailable adapter is a blocked/failed verification, never a
passing substitute. Windows GPU/UI hardware acceptance is not claimed here.

## Reference conventions

Unity's [ProBuilder cylinder documentation](https://docs.unity3d.com/Packages/com.unity.probuilder@5.0/manual/Cylinder.html)
describes radius, height, side counts and smoothing as useful authoring controls.
This example intentionally exposes a smaller fixed capped-cylinder recipe and
does not claim ProBuilder feature parity.

The [Khronos glTF 2.0 specification](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/Specification.adoc)
documents mesh normals, texture-coordinate attributes and front-face winding.
Those conventions help explain the generated presentation data; this recipe is
not glTF and does not change the existing glTF import contract.
