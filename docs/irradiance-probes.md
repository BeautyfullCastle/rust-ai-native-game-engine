# Diffuse irradiance probes

This opt-in view-layer slice of #101 adds a spatial SH9 diffuse irradiance grid
in the production Yard3D composed viewport. Authored/imported grids are joined
by a [bounded static sun-only single-bounce baker](static-diffuse-bake.md).
Reflection probes, specular IBL and broader GI remain open; direct lighting is
unchanged.

## Build and author

Enable `orr_editor/irradiance-probes` (depends on existing `models` composition).
Add `orr_editor/animated-models` for skinned receivers. Default editor/render and
simulation dependencies do not enable this path.

The Yard inspector's irradiance panel creates/opens an independent
`*.irradiance.json` sidecar. Edit grid origin, axis spacing and dimensions, select
a node and set constant linear RGB irradiance with intensity, or import full
signed SH9 JSON. Enable, save/reload, undo/redo and dirty state apply only to this
sidecar. No scene/model schema changes or simulation writes occur. A provenance
label distinguishes authored/imported data from an actual static sun bake;
baked sidecars also require a versioned fingerprint receipt.

Package imports read verified immutable assets through `Project::read_asset`
and a compiled `irradiance-probes` capability. Editing produces a user-owned
scene sidecar; installed package objects are never overwritten. Failed imports
and missing/malformed replacements preserve the last accepted data. Changing
scene or Save As retains dirty data and disables its application on a mismatched
scene rather than silently rebinding it.

## Data and shading contract

Version 1 is one axis-aligned grid, 2–4 nodes per axis (maximum 64), x-fastest:
`x + nx * (y + ny * z)`. Origins and all derived nodes must remain finite and
within ±1,000,000 world units; spacing is at least 0.001. JSON is bounded to
256 KiB; unknown fields, unsupported versions, incorrect counts, nonfinite and
out-of-range coefficients are rejected. Each node has nine **signed** linear
RGB coefficients, each component bounded to ±10,000.

The explicit real basis, evaluated at normalized world normal `(x,y,z)`, is:

1. `0.2820948`
2. `0.48860252*y`
3. `0.48860252*z`
4. `0.48860252*x`
5. `1.0925485*x*y`
6. `1.0925485*y*z`
7. `0.31539157*(3*z*z-1)`
8. `1.0925485*x*z`
9. `0.54627424*(x*x-y*y)`

These are already cosine-convolved **irradiance** coefficients, not incident
radiance SH. Imports using another basis/order/sign convention must convert
before import. Constant irradiance E uses coefficient zero `E / 0.2820948` and
all other coefficients zero. No extra convolution or gamma decoding occurs.

Each fragment interpolates signed coefficients from its eight adjacent grid
nodes, evaluates nine terms, then clamps the resulting irradiance nonnegative
once. Lambertian diffuse is `albedo * irradiance / pi`. This replaces hemisphere
**diffuse**, weighted by the distance to the nearest boundary in cell units,
clamped to [0,1]. Every face, including the maximum edge, has weight zero;
outside coverage is exact legacy fallback. A two-node axis has no full-weight
interior (its peak weight is 0.5), deliberately following the same one-cell fade.

Direct sun, its shared shadow, point light and emission are unchanged. Existing
procedural hemisphere-based specular remains a separate legacy approximation,
not probe reflections. Debug lines stay unlit. The existing single HDR display
transform remains after scene lighting; no additional tone or gamma pass.

## Resources and admission

The feature appends one packed uniform to existing globals. Its four vec4
headers plus 64 × 9 padded RGB vec4s use 9,280 bytes. Procedural globals are
replicated twice (main and existing shadow buffer); each retained static/skinned
asset has one. The viewport retains at most eight distinct model assets, so the
additional persistent uniform payload is at most 92,800 bytes; asset/format
replacement can transiently retain up to twice that payload. This excludes existing
uniform fields, model storage and driver overhead. External coordinator callers
own their renderers and must bound their lifetime caches independently.

No new RHI formats, mips, render passes, render targets or bind groups are added.
Existing per-pipeline binding layouts remain authoritative. Grid toggles and
edits create no resource variants. Without the feature the original shader
source/layout is used (only inert source comments differ). All grid admission
precedes resize, asset upload/cache change, bound changes and uniform writes.
An invalid grid combined with a resize leaves prior pixels, generations, caches
and animated bounds intact. The existing 128 MiB HDR color budget is separate.

## Verification

Run with Rust 1.97.1, release, locked/offline and a mandatory available software
GPU (no skipped adapters):

```
cargo test --release --locked --offline -p orr_render --features irradiance-probes,animation --lib
cargo test --release --locked --offline -p orr_render --features irradiance-probes,animation --test irradiance_gpu
cargo test --release --locked --offline -p orr_editor --features irradiance-probes,animated-models --lib
cargo test --release --locked --offline -p orr_editor --features irradiance-probes,animated-models --test yard3d_irradiance
cargo test --release --locked --offline -p orr_editor --features irradiance-probes --test yard3d_irradiance
```

Focused feature results are separate from default regressions and dependency
guards. Existing legacy gpu3d VIEW_FORMATS failures/ignored tests are not a full
passing baseline. Software-GPU/headless eframe coverage does not establish
native physical-window, physical-GPU or Windows-local execution.

## Sources

- [Issue #101](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/101)
- [Ramamoorthi and Hanrahan, irradiance SH representation](https://graphics.stanford.edu/papers/envmap/envmap.pdf), explicit positive real basis (equation 3) and irradiance/radiance distinction
- [Sloan, SH implementation guidance](https://www.ppsloan.org/publications/StupidSH36.pdf), low-order representation, filtering and ringing
