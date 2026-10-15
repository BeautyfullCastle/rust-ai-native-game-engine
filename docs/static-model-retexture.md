# Author a cooked static-model texture variant

This optional GPU-free authoring example creates a self-contained texture variant
of an existing cooked `orr_static_model` version-1 asset. It is a bounded part of
#99/#94: one used material receives one appended decoded PNG. Existing runtime,
importer, model format, renderer, package, scene and collision behavior is unchanged.

Save a recipe next to its source-relative input directories:

```json
{
  "format": "orr_static_material_texture",
  "version": 1,
  "source": "inputs/chair.model.json",
  "texture": "images/blue.png",
  "material": 0
}
```

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_retexture -- \
  --recipe /existing/trusted/chair-blue.retexture.json \
  --asset-id variants/chair-blue.retexture.json \
  --output /existing/trusted/chair-blue.model.json
```

The three unique option/value pairs are required and may be reordered. `--help`
or `-h` works alone. There is no force, stdin, network, install or scene-edit mode.
The existing `import` feature enables the example; without it, the executable
exits 2 with a feature diagnostic. No Cargo dependencies or features are added.

## Strict recipe and source admission

The recipe is a JSON object containing exactly the five required fields above.
Unknown or duplicate fields, missing fields, nulls, positional arrays, coercions,
negative/fractional material indices, and unsupported formats/versions reject.
The recipe is at most 16 KiB. `material` is the zero-based cooked material-table
index, not a material name, primitive index or image index. It must exist and be
used by at least one primitive. Every primitive sharing that material receives
the new image; other materials sharing its old image remain unchanged.

`source`, `texture` and the logical `--asset-id` are normalized portable ASCII
relative paths of at most 240 bytes. They allow letters, digits, `-`, `_`, `.`, and
`/`, but exclude empty/hidden/dot/traversal segments, trailing dots, backslashes,
spaces, URI syntax, percent escapes and Windows reserved device basenames. Their
suffixes must be `.model.json`, `.png` and `.retexture.json` respectively. Direct
input paths and recipe identity must be distinct. All recipe and identity checks
finish before resource resolution; source is resolved once before texture.

The entire cooked source is admitted through the unchanged `StaticModel` loader
before selecting a material. Invalid unrelated primitives, materials, images,
transforms or provenance cannot become acceptable by selecting another slot.
The source need not be created by the extraction example, but must be a valid
cooked model. The source's existing resource metadata is retained transitively
through the exact cooked-input hash.

## Preservation and identity

The result keeps primitive traversal order, vertices, triangle indices, normals,
UVs, fully composed transforms and material indices. There is no centering,
pivot edit, hierarchy change, coordinate conversion or generated collision.
Mirrored and nonuniform transforms preserve existing normal/winding behavior.

Every original material and image retains its table position, including unused
entries. The new image is appended without replacing, deduplicating or reclaiming
images. Only the selected material's image index changes. Its base-color factor,
alpha, linear/nearest filtering and wrap modes remain unchanged. PNG alpha is
preserved in cooked data; the existing renderer remains opaque.

The result's logical `asset_id` is the recipe asset ID. Every primitive keeps the
original suffix beginning `#node=`, prefixed by that recipe identity. Give each
variant a distinct logical recipe ID so consumers can cache and bind multiple
variants without identity collisions. Output filenames alone do not supply this
namespace.

Exactly three ordered dependencies identify the bytes used:

1. `$source`: SHA-256 of the exact recipe bytes, including whitespace
2. The recipe's source path: SHA-256 of the exact cooked input bytes
3. The recipe's texture path: SHA-256 of the exact PNG input bytes

The old dependency list is not copied alongside a conflicting `$source` entry.
Its data is bound by the direct cooked-source digest. Reformatting any input can
therefore change provenance even when decoded geometry or pixels are unchanged.
The result is validated, serialized within the cooked limit, freshly reloaded,
and compared in full before it is returned or published.

## PNG and memory limits

PNG admission matches the existing importer: expansion and 16-bit stripping must
produce exact RGBA8, with dimensions from 1 through 2,048 on each axis. RGBA PNG
and indexed PNG with alpha can qualify; RGB-only and grayscale images do not get
an implicit alpha fallback. APNG is unsupported. The bounded decoder reads through
PNG finish, including rejection of truncated endings and critical-chunk CRC
failures. Admission follows the existing importer's PNG decoder policy rather
than claiming full PNG conformance: ancillary metadata with a bad CRC can be
discarded while valid decoded pixels are retained. The exact PNG bytes, including
ignored metadata, are still bound by the texture dependency digest.

Recipe plus source bytes plus PNG bytes must total at most 64 MiB. The original
source must leave an unused slot under the 32-image limit, and original decoded
images plus the appended RGBA8 image must fit the 32 MiB decoded-image budget.
Existing primitive, vertex, index and material limits remain. The resulting cooked
JSON must independently fit 64 MiB; retaining original images can make it larger
than the source. Repeated authoring is not a texture-memory optimization.

## Filesystem and publication contract

The recipe's admitted parent anchors both resource paths. Filesystem recipe and
ancestor names may be non-UTF8 where the OS supports them; logical IDs and portable
input references must be UTF8 ASCII. All inputs must be bounded regular files.
The positive arbitrary-byte filename fixture is Linux-only. macOS filesystems can
reject such names before the CLI runs, so that filesystem capability is not
applicable there; rejection of non-UTF8 option names and logical IDs is separately
tested on Unix using ordinary valid filesystem paths.
Opened-file identity/length/modification checks and bounded reads reject observed
changes. Empty paths, literal parent traversal, symlink paths/ancestry, non-directory
ancestors, trailing-dot/space components and nonlocal Windows path prefixes reject.

Parents must be trusted and stable. These portable checks are not a capability
sandbox against hostile same-user/root concurrent replacement or in-place
mutation, and a path replacement can race open, including substituting a FIFO.
Filesystem paths can resolve onto mounted storage; callers must supply trusted
local directories.

The output parent must already exist. The output basename is portable ASCII,
at most 100 bytes, ends in `.model.json`, and excludes Windows device names.
Any lexical `.orr` ancestor, case-insensitively, rejects. Trusted callers must not
use short-name aliases, alternate directory aliases or mounts to reach an installed
package store. Existing outputs, hard-linked source aliases, directories and
dangling symlinks are never replaced.

An owned, uniquely created sibling stage is written, synced and read back, then
hard-linked to the absent destination. Hard-link support is required; there is
no copy/replace fallback. Cleanup touches only the owned stage. Failure before
publication leaves no new destination. After publication, stage-cleanup or
directory-sync problems produce warnings and retain the result. Non-Unix directory
sync is unavailable through this implementation; no power-loss guarantee is made.
Exit 0 means help or publication, with any warnings relevant. Exit 2 means error
and no destination was published by that operation.

## Existing consumer route and verification

Include the resulting cooked model and applicable licensing notices in a newly
authored package/version, then use existing install, Load/Assign and save-binding
steps. The tool does not mutate installed assets, package locks, scenes or collision.
The cooked output includes decoded pixels and can load without the recipe, original
cooked input or PNG; preserve those inputs separately for reproducible authoring.

Model tests cover closed recipes, resource admission/order, exact provenance,
identity, preservation, PNG parity, budgets and CLI failure/publication behavior.
The platform-active model inventories are 28 tests on Linux, 27 on macOS and 26 on
Windows, from 29 declarations. A platform-inapplicable filesystem fixture is not
counted as a pass or an ignored test.
Bindings acceptance loads distinct variants from the existing immutable package
route. Software-GPU acceptance checks changed selected material and unchanged
unselected material against independent geometry/UV/transform/material controls.
Room export acceptance uses current-source authoring, exporter and runtime producers,
checks exact exported model inventory, and validates original/export behavior with
source inputs hidden. Mandatory gates invoke ignored consumer/device/export tests
explicitly; a listing or ignored inventory is not acceptance.

```sh
cargo test --locked --offline --release -p orr_model --features import \
  --test static_model_retexture
cargo clippy --locked --offline --release -p orr_model --features import \
  --all-targets -- -D warnings
```

The existing mandatory `sample-build` lane runs the source-bound CPU, software-GPU
and Room-export gates. Protected Room checks are unchanged. Local model-only offline
validation is separate evidence and does not establish bindings, GPU, export,
native-window, Windows/macOS or physical-device acceptance. Full hosted checks must
pass for the exact reviewed head before publication is considered verified.
