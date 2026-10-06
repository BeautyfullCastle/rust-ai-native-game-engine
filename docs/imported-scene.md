# Imported scene composition and one point light

`orr_render`'s optional `imported-scene` feature composes opaque imported static
models in one frame. With `animation` enabled, imported skinned models join the
same camera, lighting, color target, and depth buffer. This is a bounded milestone
for [#101](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/101),
which remains open for its broader renderer scope.

## Run the package-backed example

From the repository root, install both original CC0 asset packages into a new
content project. No network, renderer-generated replacement geometry, or package
script execution is involved:

```sh
mkdir -p /tmp/imported-scene-project
cargo run -p orr_package --bin orr_pkg -- install /tmp/imported-scene-project \
  --path assets/imported_scene_demo --path assets/animation_demo
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project
```

The scene contains an imported textured background wall/floor, the existing
animated two-joint strip, and an imported textured foreground crossbar/post. The
crossbar obscures part of the strip, and the strip appears in front of the wall.
`Scene::draw` is the shared frame path for both the native window and offscreen
capture. It submits `ImportedBatch` values to `ImportedSceneRenderer`; it never
calls the procedural `Renderer3D` or fabricates a runtime fallback model.

The native window's controls are:

- W/A/S/D: move the point light along world Y/X in 0.25-unit steps
- Q/E: move it along world Z; moving an off light enables it
- P: toggle the point light; off/on preserves its position during the session
- Space: pause/resume animation; Left/Right: seek by 0.1 seconds
- 1/2: play the strip's `bend`/`pulse` clip, looping
- F5: save light settings; F9: reload the same settings file
- Escape: close; resizing keeps the same imported scene and camera

A missing package, invalid package digest, unsupported source, or failed cooked
reload is an error. The example prints the package installation command for
missing package/project content; it does not display replacement geometry.

## Headless capture and repeatable comparisons

`--offscreen` writes a binary PPM from actual GPU readback and opens no window.
The example prints the selected adapter and whether it is software. PPM is
uncompressed RGB with sRGB-encoded bytes, so pixel comparisons should use the
same backend/adapter; these are presentation captures, not simulation checksums.

```sh
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --offscreen /tmp/lit-scene.ppm --seek 0.5 --point on \
  --light-position 0.6,2.2,2.5

# Same content/time, different submission order: opaque overlap remains correct.
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --offscreen /tmp/lit-scene-reversed.ppm --seek 0.5 --point on \
  --light-position 0.6,2.2,2.5 --reverse-order
cmp /tmp/lit-scene.ppm /tmp/lit-scene-reversed.ppm

# Light-off, moved-light, and different-pose captures use that same draw path.
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --offscreen /tmp/lit-scene-off.ppm --seek 0.5 --point off
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --offscreen /tmp/lit-scene-moved.ppm --seek 0.5 --light-position -2,1,2
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --offscreen /tmp/lit-scene-pose.ppm --seek 1.0 --point on
```

Arguments use separate tokens; negative position values are accepted. The available
flags are `--project PATH`, `--offscreen PATH`, `--size WIDTHxHEIGHT`, `--seek SECONDS`,
`--point on|off`, `--light-position X,Y,Z`, `--settings PATH`, `--save-settings`,
`--reverse-order`, and `--help`. `--size` defaults to `960x640` and accepts
`1..=8192` per axis. Headless mode renders the exact seek time without advancing
it; the native window starts there and advances playback. `--point` is applied
after the position override, so `--point off` always disables the light.

Reversing order is not a promise for coplanar surfaces: equal-depth ties can still
be order-dependent. The fixture surfaces have distinct depths.

## User-owned, versioned light settings

By default the editable file is
`PROJECT/lit-imported-scene-light.json`, outside the immutable installed package
objects in `PROJECT/.orr/packages/objects`. Use `--settings PATH` to select
another user-owned file whose parent directory already exists. Package-state
paths, symlink files, and parent-directory aliases into the project's package
state are rejected. The example rechecks the path before each save and reload.

If no settings file exists, the demonstration explicitly starts with the warm
point light enabled. This is only an example choice: `PointLightSettings::default()`
is off, and existing standalone static/skinned draws keep their previous lighting.
An existing invalid settings file fails startup rather than silently replacing it.
F9 failure reports an error while the last valid in-memory lighting keeps drawing.

The file format is strict JSON, at most 4096 bytes:

```json
{
  "version": 1,
  "point_light": {
    "position": [0.6, 2.2, 2.5],
    "color": [1.0, 0.82, 0.58],
    "intensity": 2.8,
    "range": 6.0
  }
}
```

Off is `{ "version": 1, "point_light": null }`. Missing/unknown/duplicate fields,
unsupported versions, malformed or oversized JSON, and invalid light values are
rejected. Position components must be finite within `[-1e9, 1e9]`; linear RGB and
intensity must be finite within `[0, 1e4]`; range must be finite within
`[1e-6, 1e9]`. Zero intensity is allowed and contributes nothing.

`PointLightSettings::{from_bytes,to_bytes,load,save,reload}` share validation.
`reload` replaces the complete in-memory settings only after successful loading.
`save` stages a sibling file and replaces the destination; it never writes model
or package metadata. F5 and `--save-settings` explicitly save; simply running or
moving the light does not change files.

For a CLI round trip, save the settings once, then capture again using the same
file and pose without overrides:

```sh
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --settings /tmp/imported-scene-project/light.json --save-settings \
  --light-position 1.4,2.0,2.5 --point on --seek 0.5 --offscreen /tmp/saved.ppm
cargo run -p orr_render --features imported-scene,animation \
  --example lit_imported_scene -- --project /tmp/imported-scene-project \
  --settings /tmp/imported-scene-project/light.json \
  --seek 0.5 --offscreen /tmp/reloaded.ppm
cmp /tmp/saved.ppm /tmp/reloaded.ppm
cargo run -p orr_package --bin orr_pkg -- verify /tmp/imported-scene-project
```

## Lighting contract

There is at most one unshadowed point light. For world-space shaded position
`p`, light position `q`, distance `d = length(q - p)`, positive range `r`, unit
surface normal `n`, and `L = (q - p) / d`, its added linear RGB diffuse lighting is:

```text
color * intensity * max(dot(n, L), 0) * (max(1 - distance / range, 0)^2)
```

At `distance <= 1e-6` its contribution is explicitly zero. At or beyond the
range it is also zero. This finite-range falloff is a stylized diffuse convention;
intensity has no photometric-unit or inverse-square claim. RGB is linear, and
position/range use world units.

Static primitives use their imported world transforms and inverse-transpose
normal transforms. Animated primitives use the existing skinned, instance-world
positions and normals. Both receive the same point-light parameters. The point
term is added to the existing hemisphere ambient and directional diffuse terms,
then multiplied by decoded base-color texture/material color and exposure, with
the existing optional tone mapping and output encoding. The example turns tone
mapping off and retains a dim ambient/directional baseline so light-off content
is still visible.

## Library integration and frame ownership

Enable `imported-scene` for static composition; add `animation` for skinned
batches. `imported-scene` implies `models`. Use one coordinator per color-target
format, with model renderers created on the same RHI device:

```rust,ignore
let mut scene = ImportedSceneRenderer::new(rhi.clone(), format)?;
let mut batches = [
    ImportedBatch::Static(&mut background),
    ImportedBatch::Skinned { renderer: &mut animated, instances: &instances },
    ImportedBatch::Static(&mut foreground),
];
scene.draw(
    ImportedSceneTarget { view, size, format, sample_count: 1 },
    &camera,
    &lighting, // shadows must be false
    &point_settings,
    &mut batches,
)?;
```

The caller supplies accurate target metadata. The RHI cannot introspect a view
to verify its actual format, dimensions, sample count, or device identity. A
renderer can occur once per frame through the exclusive mutable batch borrow;
put multiple instances of one animated asset in one skinned batch.

The coordinator validates the complete frame before frame-side GPU allocation,
uniform writes, encoding, submission, or animated submitted-bounds changes. It
checks dimensions, format agreement, a single sample, camera and baseline light,
point settings, animation poses/instance transforms, and frame budgets (at most
256 batches and 4096 combined primitive/instance draws). A rejected frame leaves
the prior accepted depth generation and submitted animated bounds unchanged.

An accepted frame has one coordinator-owned `Depth32Float` attachment, shared by
all batches. Color/depth are cleared once, preserved across subsequent batches,
and submitted in one command encoder. Renderer-local standalone clear settings
are ignored. Empty frames still clear. Depth storage is reused at the same size
and replaced on accepted resizes; `depth_size()` and `depth_generation()` expose
that lifecycle. A minimized native window skips zero-sized draws.

## Bounds of this milestone

This path supports opaque imported static/skinned base-color materials,
hemisphere ambient, directional diffuse, and one unshadowed point light at a
single sample. It does not add shadows, spot lights, HDR output, bloom, probes,
PBR, MSAA, glTF punctual-light import, or shared depth with the procedural
`Renderer3D` path. Existing standalone draw entry points remain available and
continue owning their own depth targets. No simulation state, inputs, rollback,
or golden checksum expectations are changed.

The original static fixtures and their reproducible Python authoring script are
in `assets/imported_scene_demo`; the animated asset remains in
`assets/animation_demo`. Their license files record provenance. The authoring
script is optional offline tooling, not part of the runtime or install path.
