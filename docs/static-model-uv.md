# Author a cooked static-model UV variant

This optional GPU-free authoring example transforms existing UV0 coordinates in
one used material slot of an admitted cooked `orr_static_model` version-1 asset.
It creates a separate ordinary cooked model for explicit package installation and
scene assignment. It adds no runtime, importer, editor or model-format API.

Save a recipe next to its source-relative input directory:

```json
{
  "format": "orr_static_material_uv",
  "version": 1,
  "source": "inputs/chair.model.json",
  "material": 0,
  "offset": [0.25, -0.5],
  "scale": [2.0, 0.5],
  "quarter_turns": 1
}
```

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_uv -- \
  --recipe /existing/trusted/chair-tile.uv.json \
  --asset-id variants/chair-tile.uv.json \
  --output /existing/trusted/chair-tile.model.json
```

Exactly three unique option/value pairs are required, in any order. `--help` or
`-h` works alone. There is no force, stdin, network, install or scene-edit mode.
The existing optional `import` feature provides the dependencies; no Cargo feature
or dependency is added. Without it, the executable exits 2 with a diagnostic.

## Closed recipe and cooked-input boundary

The recipe is a JSON object with exactly the seven required fields above. Unknown,
duplicate, missing or null fields, positional object aliases, coercions and
unsupported versions reject. `material` is a nonnegative integer cooked-table
index that exists and is used by a primitive. It is not a primitive or image index.
`offset` and `scale` are exactly two finite f32 values, with both scale components
strictly positive. `quarter_turns` is an integer from 0 through 3.

The recipe is at most 16 KiB, and exact recipe bytes plus exact cooked-source bytes
must total at most 64 MiB. The whole cooked source is admitted before material
selection, including unrelated geometry, materials, transforms, images and
provenance. All existing model limits and the independent 64 MiB cooked-output
limit remain in effect. There is no new image allocation or decoding step.

`source` and logical `--asset-id` are normalized portable ASCII relative paths of
at most 240 bytes ending in `.model.json` and `.uv.json` respectively. Allowed
characters are letters, digits, `-`, `_`, `.`, and `/`. Empty, hidden, dot or traversal
segments, trailing dots, backslashes, spaces, URI syntax, percent escapes and
Windows device basenames reject. Recipe identity and input path must be distinct
case-insensitively. All recipe checks finish before the source is resolved exactly
once. Existing dependency URIs are never opened.

This tool accepts cooked input only. If an importer has already baked texture
coordinates, those values are its input; no glTF extension or original source is
reopened or interpreted. It does not add UV unwrapping, seam editing, per-face
selection, additional UV channels, negative-scale mirroring or arbitrary angles.

## Exact transform and preservation

For each input UV `(u,v)`, compute `x = u * scale[0]`, `y = v * scale[1]`, then:

| quarter_turns | Rotated pair |
|---|---|
| 0 | `(x,y)` |
| 1 | `(-y,x)` |
| 2 | `(-x,-y)` |
| 3 | `(y,-x)` |

Finally add the two offset components. Rotation is around the UV origin `(0,0)`;
scale, rotation and offset do not commute. For example, `(u,v)=(0.25,0.75)` with
the recipe above gives scaled `(0.5,0.375)`, rotated `(-0.375,0.5)` and final
`(-0.125,0.0)`. Arithmetic intermediates must remain finite, and final coordinates
must remain within the existing inclusive absolute component limit of 65,536.

There is no clamp to `[0,1]`. Negative and out-of-range coordinates continue to use
the preserved clamp/repeat/mirror addressing and nearest/linear filtering. Identity
parameters (scale `[1,1]`, zero turns and zero offsets of either sign) bypass
arithmetic, preserving original UV bits including signed zero.

Every primitive that uses the chosen material changes. A separate material that
shares its image remains untouched. Positions, normals, indices, primitive order,
world transforms, material indices, all material and image data, sampler modes,
base-color factors and unselected UV bits are preserved. Materials and images are
not cloned, appended, removed or reordered. The renderer's opacity remains unchanged.

## Identity and provenance

The output `asset_id` is the logical recipe ID. Each primitive retains its original
suffix beginning `#node=`, now prefixed by that recipe ID. Use a different logical
ID for each separately bindable variant; a different filename alone does not
supply a new identity.

The output has exactly two ordered dependencies:

1. `$source`: SHA-256 of the exact recipe bytes, including whitespace
2. The normalized source path: SHA-256 of the exact cooked input bytes

Old dependency metadata is bound transitively by the cooked-source digest rather
than copied or resolved. Reformatting either input can change provenance while
leaving decoded data unchanged. The result is validated, cooked, freshly loaded
and compared, including exact reserialization, before it is returned or published.

## Filesystem and publication

The admitted recipe parent anchors the source reference. Filesystem input and
ancestor names can be non-UTF8 where supported; logical IDs and source references
remain UTF8 ASCII. Positive arbitrary-byte filename coverage is Linux-only because
macOS filesystems can reject the fixture before the CLI runs. Unix logical-ID and
option-name rejection is separate and uses ordinary valid filesystem paths.

Inputs must be bounded regular files. Opened-file identity, length, modification
checks and bounded reads reject observed changes. Empty paths, literal parent
traversal, symlink paths or ancestors, nondirectory ancestors, trailing-dot/space
components and nonlocal Windows prefixes reject. Parents must be trusted and
stable: these portable checks are not a capability sandbox against hostile
concurrent replacement or in-place mutation. Mounted storage and aliases remain
the trusted caller's responsibility; a replacement can race open, including FIFO
substitution.

The output parent must exist. Its basename is portable ASCII, at most 100 bytes,
ends in `.model.json`, and excludes device names. Any lexical `.orr` ancestor,
case-insensitively, rejects. Do not use alternate aliases or mounts to address an
installed package store. Existing outputs, directories, hard-linked source aliases
and dangling symlinks are never replaced.

A uniquely created owned sibling stage is written, synced and read back, then
hard-linked to the absent destination. Hard-link support is required; there is no
copy/replace fallback. Cleanup affects only that stage. Failure before publication
leaves no new destination. After publication, stage cleanup or directory sync
problems warn and retain the result. Non-Unix directory sync is unavailable; no
power-loss guarantee is made. Exit 0 means help or publication (check warnings);
exit 2 means error and no output was published by that operation.

## Consumer route and acceptance

Include the cooked variant and applicable notices in a new package/version, then
use existing installation, Load/Assign and saved-binding operations. The tool does
not mutate installed assets, locks, scenes, collision or simulation. Its output
includes the original decoded images and loads without the recipe or original
cooked input; preserve those separately for reproducible authoring.

Focused tests cover strict recipes, full-source admission, arithmetic and boundary
oracles, shared material/image isolation, bit preservation, provenance and CLI
failure/publication paths. Installed bindings acceptance creates distinct assets,
assigns them and saves/reopens bindings. Software-GPU acceptance uses independent
hand-calculated UVs and negative controls for addressing/filtering and unrelated
materials. Source-hidden read-only Room export acceptance uses current-source
producer binaries and compares original/export rendering and simulation frames.
Ignored consumer tests must be invoked explicitly by mandatory gates; an ignored
listing is not acceptance.

```sh
cargo test --locked --offline --release -p orr_model --features import \
  --test static_model_uv
cargo clippy --locked --offline --release -p orr_model --features import \
  --all-targets -- -D warnings
```

Local offline model-only checks are separate from installed-consumer, software-GPU,
source-hidden export and native-platform acceptance. Fresh exact-head required CI
and independent review are required before development integration. No full-workspace,
native-window or physical-device acceptance follows from these focused checks.
This is one bounded authoring increment; #99/#94 remain broader open work.
