# Generate planar UVs for a cooked static model

The optional, GPU-free `static_model_planar_uv` example generates UV0 from vertex
positions for one used material slot of an admitted `orr_static_model` version-1
asset. Unlike `static_model_uv`, it can spread an input whose UVs are all `[0,0]`
across a texture. The output is an ordinary cooked model for explicit package
installation and scene assignment. No runtime, importer, model schema, shader,
editor API, dependency or Cargo feature is added.

Save a recipe beside its source-relative input directory:

```json
{
  "format": "orr_static_planar_uv",
  "version": 1,
  "source": "inputs/floor.model.json",
  "material": 0,
  "plane": "xz",
  "origin": [0.0, 0.0, 0.0],
  "units_per_repeat": [2.0, 2.0]
}
```

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_planar_uv -- \
  --recipe /existing/trusted/floor.planar-uv.json \
  --asset-id variants/floor.planar-uv.json \
  --output /existing/trusted/floor.model.json
```

Exactly three unique option/value pairs are required, in any order. `--help` or
`-h` works alone. Without `import`, the executable exits 2 immediately, including
for help, before touching any file. There is no force, stdin, network, install or
scene-edit mode. Filesystem operations currently require Linux x86_64; the pure
resolver-based authoring helper is portable.

## Closed input boundary

The recipe is a JSON object with exactly the seven required fields shown above.
Unknown, duplicate, missing, null, positional aliases, coercions and unsupported
formats/versions reject. `material` is a nonnegative integer index into the cooked
material table; it must exist and be used by at least one primitive. `plane` is
exactly `xy`, `xz` or `yz`. `origin` has exactly three finite f32 components, and
`units_per_repeat` exactly two positive finite f32 components. Parameters are first
parsed as f32, then widened for arithmetic; they are not arbitrary-precision JSON
numbers. Zero or negative density rejects, including negative zero.

The recipe is limited to 16 KiB. Exact recipe bytes plus exact cooked-source bytes
must total at most 64 MiB; the cooked output independently retains its existing
64 MiB limit. Full source-model admission precedes material selection, including
unselected geometry, transforms, images, materials and dependency metadata. No
image is decoded and no source dependency is followed.

`source` and logical `--asset-id` are normalized portable ASCII relative paths of
at most 240 bytes ending in `.model.json` and `.planar-uv.json`, respectively.
Letters, digits, `-`, `_`, `.`, and `/` are allowed. Empty/hidden/traversal segments,
trailing dots, backslashes, spaces, URI syntax, percent escapes and Windows device
basenames reject. Recipe identity and direct input path must be distinct
case-insensitively. Recipe admission finishes before resolving the source exactly
once. The CLI anchors that reference to the recipe's parent directory.

## Exact projection contract

Positions are projected in the **model's coordinates**, including the primitive's
existing composed ancestor transform but excluding any scene entity/binding
transform. The existing column-major matrix has columns `m[0]` through `m[3]`.
Existing model admission also retains its f32 world-position bounds, independent
of this tool's f64 projection arithmetic. Full model admission requires its bottom row to be exactly `[0,0,0,1]`; there is
no perspective divide.

For each axis `r` in x, y, z, widen each f32 input to f64 and compute in this order:

1. `a = m[0][r] * position[0]`
2. `b = m[1][r] * position[1]`
3. `c = m[2][r] * position[2]`
4. `ab = a + b`, then `abc = ab + c`, then `world[r] = abc + m[3][r]`

These are separate ordered multiply/add operations, not fused `mul_add`.
Choose axes `(x,y)` for `xy`, `(x,z)` for `xz`, or `(y,z)` for `yz`.
For each output component, subtract that axis's widened origin from `world`,
then divide by the corresponding widened `units_per_repeat`. Check the f64
result is finite and within inclusive ±65,536 before rounding to f32, and check
the rounded f32 again. A value just outside the bound cannot round back inside
and be accepted. Existing input bounds keep the model-position f64 arithmetic
finite; all three model coordinates are checked for finiteness as well. No clamping, normalization, axis flipping, automatic fit or UV wrapping
is applied. For the example, model position `[6,5,8]` produces UV `[3,4]`.

All primitives sharing the selected material receive newly computed UV0, even
when the old UVs were already usable. Unselected UV bits, including signed zero,
are unchanged. Positions, normals, indices, primitive order, transforms, material
indices, all material/image data and sampler modes are preserved. A different
material sharing the same image remains unchanged. Negative and out-of-range
UVs use the existing clamp/repeat/mirror addressing and nearest/linear filtering.

This is **planar projection, not general UV unwrapping**. Overlapping projected
faces overlap in UV space; faces perpendicular to the plane can collapse to
lines or points. It generates no seams, splits no topology, adds no UV channels,
and does not select individual faces or choose a plane automatically. It is best
suited to floors, walls and planar props. Apply the existing UV transform,
retexture and sampler examples explicitly afterward if desired, supplying each
new cooked result as the next input.

## Identity and provenance

The output `asset_id` is the logical recipe identity. Each primitive retains its
original suffix beginning `#node=`, now prefixed with that recipe identity. Use a
different logical ID for each separately bindable variant.

Exactly two ordered dependencies replace the old dependency list:

1. `$source`: SHA-256 of the exact recipe bytes, including whitespace.
2. The recipe's normalized `source` path: SHA-256 of the exact cooked input bytes.

Old metadata is bound transitively by the cooked-source digest, not copied or
opened. The result is validated, serialized, freshly loaded and checked for
source equality and byte-identical reserialization before return/publication.

## Filesystem and publication

Paths must have existing trusted, stable, link-free ancestry. Filesystem paths
are UTF-8. Empty paths, parent traversal, platform prefixes, symlink paths or
ancestors, nondirectory ancestors, trailing-dot/space components and reserved
device names reject. Output cannot be inside a lexical `.orr` package store.
This is not a capability sandbox against concurrent hostile directory replacement;
do not use alternate directory aliases or mounts to address a package store.

Inputs must be bounded regular files. The Linux x86_64 open uses `O_NOFOLLOW` and
`O_NONBLOCK`, preventing final-symlink traversal and FIFO-replacement blocking.
Before/open/after identity, length and modification checks, bounded reads and
ancestry rechecks reject observed replacement or mutation. Other platforms reject
filesystem input rather than silently falling back to less safe opens.

The output filename is portable ASCII, at most 100 bytes, and ends in
`.model.json`. Existing files, directories, symlinks and hard links are never
overwritten. A private create-new stage in the destination directory is written,
synced and read back. Its identity is checked before atomic no-replace hard-link
publication and before cleanup; a replacement entry is never deliberately removed.
A filesystem without hard-link publication fails without overwrite fallback.
Cleanup/directory-sync failures after publication are warnings, not claims that
publication failed. Exit 0 means help or publication (read warnings); exit 2 means
error/not published. This tool never installs a package or changes a scene.

## Selected local verification

The selected local acceptance run on 2026-10-20 passed against base
`8876dbc7b618a3e105458b6d966b68b952b8e5f7` plus this change:

- Four new CPU contract tests and one separately invoked actual-CLI test passed.
  The retained regressions passed: 21 existing UV tests and one import-enabled
  model-library test.
- Selected formatting and strict lint checks passed for default-off, model/import
  and consumer targets. The actual feature-off executable returned exit 2 for
  `--help` with the required-feature diagnostic; the import-enabled producer was
  preserved separately before building that feature-off executable.
- One renderer acceptance test and its independent receipt validation passed.
  The independent projection/texture oracle covered 6,906 selected pixels and
  all 16 texture texels; 721 unselected pixels were identical. Each of five wrong
  controls differed from the projected result by at least 5,144 pixels. These
  include the collapsed existing-UV affine control, which does not recover
  spatial UVs from an all-zero input.
- One editor acceptance test and its independent receipt validation passed.
  Actual CLI bytes were installed, then consumed through real editor widgets:
  Load, Assign, Undo, Redo, Save and fresh reopen. The recipe, cooked source,
  original CLI output and package source were hidden before later consumption.
  Room Play reached tick 12 with presentation-off/on authoritative Frame parity:
  2,381 identical bytes and checksum `1978905972186806402`. The viewport changed
  by 1,495 pixels when presentation was enabled.
- Fourteen editor PNG/raw capture pairs (seven phases, each with full-UI and
  viewport captures), plus seven separate renderer PNG/raw pairs, passed exact
  decoded-pixel equality checks: 21 pairs in total. This exact capture comparison
  is distinct from the renderer's documented pixel-oracle color tolerance.

The output remains a new `.model.json` file. Existing catalog discovery uses the
supported `.json` extension plus the cooked format header; changing only a file
extension is not a new catalog feature. Package installation and scene assignment
in these tests are separate existing consumer operations, not CLI side effects.

The acceptance record retains the original formatting/lint failures, the missing
sidecar/fixture-order failure, unsupported catalog-filename failure and UI PNG
premultiplication failure. The corresponding owner-reviewed fixes corrected test
formatting, test-local lint annotations, fixture setup and lossless capture; they
did not weaken product admission or validator tolerances. Earlier failed evidence
was retained rather than overwritten.

This evidence is local Linux x86_64 with SwiftShader Device (Subzero), Vulkan CPU,
using real egui/GPU rendering. It does not establish native OS input, Windows,
hosted CI, the full workspace matrix, general UV unwrapping or export certification.
Any integration into an existing CI job and its exact-head hosted result remain
separate from this local acceptance; no new workflow matrix is claimed here.
