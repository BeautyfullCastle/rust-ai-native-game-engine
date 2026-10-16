# Author a cooked static-model pivot variant

This optional GPU-free authoring example rebases every primitive of an existing
cooked `orr_static_model` version-1 asset around one explicit model-space pivot.
It is a bounded part of #99/#94. Existing runtime, importer, model format,
renderer, package, scene and collision behavior is unchanged.

Save a recipe next to its source-relative input directory:

```json
{
  "format": "orr_static_model_pivot",
  "version": 1,
  "source": "inputs/chair.model.json",
  "pivot": [2.0, -3.0, 1.5]
}
```

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_pivot -- \
  --recipe /existing/trusted/chair.pivot.json \
  --asset-id variants/chair.pivot.json \
  --output /existing/trusted/chair.model.json
```

The three unique option/value pairs are required and may be reordered. `--help`
works alone; `-h` and additional options reject. There is no force, stdin,
network, install or scene-edit mode. The existing `import` feature enables the
example; without it, the executable exits 2 with a feature diagnostic. No Cargo
dependencies or features are added.

## Strict recipe and source admission

The recipe is a JSON object containing exactly the four required fields above.
Unknown or duplicate fields, missing fields, nulls, positional arrays, coercions,
unsupported formats/versions and malformed JSON reject. `version` must be the
integer `1`. `pivot` must be an array of exactly three JSON numbers. They are
converted to `f32` before finite and inclusive +/-1,000,000 range checks. Decimal
rounding is therefore observable: `1000000.01` rounds to `1000000.0` and qualifies;
`1000000.1` does not. Tiny values may become subnormal or zero in `f32`. Strings,
booleans, coordinate objects, NaN and infinity are unsupported. There is no
implicit default, automatic center, bounding-box center or per-primitive pivot.
The recipe is at most 16 KiB, including whitespace.

The recipe's `source` and logical `--asset-id` are normalized portable ASCII
relative paths of at most 240 bytes. They allow letters, digits, `-`, `_`, `.`,
and `/`, but exclude empty/hidden/dot/traversal segments, trailing dots,
backslashes, spaces, URI syntax, percent escapes and Windows reserved device
basenames. Their suffixes must be `.model.json` and `.pivot.json` respectively.
The direct input path and recipe identity must be distinct. All recipe and
identity checks finish before resource resolution. The cooked source resolves
exactly once; historical dependencies are not resolved.

The entire source is admitted through the unchanged `StaticModel` cooked loader
before any transform changes. Invalid unrelated primitives, unused materials or
images, transforms and provenance reject. Raw glTF/GLB is not an input. The
source need not be created by the extraction example, but must be a valid cooked
model. Its existing dependency metadata is retained transitively through the
exact cooked-input hash.

## Model-space rebasing and preservation

For every primitive's fully composed column-major affine matrix `M`, the result
is `T(-pivot) * M`. Only `transform[3][0..3]` changes: each translation component
is its original `f32` value minus that pivot component using `f32` arithmetic.
There is no rotation, scaling, coordinate conversion, matrix decomposition,
vertex baking, hierarchy edit, per-node local translation or collision change.
For example, origin `[4, -7, 11]` and pivot `[2, -3, 1.5]` produce
`[2, -4, 9.5]`, including for mirrored and nonuniform linear transforms.

Each nonzero-axis result must be finite and within the existing inclusive
+/-1,000,000 transform bounds. Overflow or an out-of-range result rejects rather
than clamping. For either sign of zero pivot, that axis skips arithmetic entirely,
preserving the original translation bits including signed zero. All other
transform bits, geometry positions, normals, UVs, indices, material indices,
primitive traversal order, material/image table positions, base-color/alpha
values, decoded pixels, filtering and wrap modes are preserved. Existing unused
entries remain. Mirrors retain the same determinant and existing normal/winding
behavior. The renderer still treats alpha as opaque.

Moving the visual origin does not edit scene placements or physics. Existing
bindings position the rebased visual through the usual model-instance transform;
author any desired placement or collision changes separately. This tool makes
no claim that the chosen visual pivot is a center of mass or collision origin.

The result's logical `asset_id` is the recipe asset ID. Every primitive keeps its
original suffix beginning `#node=`, prefixed by that recipe identity. Give each
variant a distinct logical recipe ID so consumers can cache and bind multiple
variants without identity collisions. Output filenames alone do not supply this
namespace.

Exactly two ordered dependencies identify the bytes used:

1. `$source`: SHA-256 of the exact recipe bytes, including whitespace
2. The recipe's source path: SHA-256 of the exact cooked input bytes

The old dependency list is not copied alongside a conflicting `$source` entry.
Its data is bound by the direct cooked-source digest. Reformatting either input
can therefore change provenance even when the decoded result is unchanged.
The result is validated, serialized within the cooked limit, freshly reloaded,
and compared in full before it is returned or published. Validation includes
unchanged linear normal-transform rules and the world-vertex range after rebasing.

## Memory and cooked limits

Recipe plus cooked-source bytes must total at most 64 MiB. The resulting cooked
JSON must independently fit 64 MiB; a longer primitive namespace can exceed that
output budget even when the admitted source fits. Existing primitive, vertex,
index, image, material and 32 MiB decoded-image limits remain. Rebasing does not
reclaim memory, deduplicate resources or optimize textures.

## Filesystem and publication contract

The recipe's admitted parent anchors the normalized cooked-source path.
Filesystem recipe and ancestor names may be non-UTF8 where the OS supports them;
logical IDs and portable input references must be UTF8 ASCII. All inputs must be
bounded regular files. The positive arbitrary-byte filename fixture is
Linux-only. macOS filesystems can reject such names before the CLI runs, so that
filesystem capability is not applicable there; rejection of non-UTF8 option names
and logical IDs is separately tested on Unix using valid filesystem paths.
Opened-file identity/length/modification checks and bounded reads reject observed
changes. Empty paths, literal parent traversal, symlink paths/ancestry,
non-directory ancestors, trailing-dot/space components and nonlocal Windows path
prefixes reject.

Parents must be trusted and stable. These portable checks are not a capability
sandbox against hostile same-user/root concurrent replacement or in-place
mutation, and a path replacement can race open, including substituting a FIFO.
Filesystem paths can resolve onto mounted storage; callers must supply trusted
local directories.

The output parent must already exist. The output basename is portable ASCII,
at most 100 bytes, ends in `.model.json`, and excludes Windows device names.
Any lexical `.orr` ancestor, case-insensitively, rejects. Trusted callers must not
use short-name aliases, alternate directory aliases or mounts to reach an
installed package store. Existing outputs, hard-linked source aliases,
directories and dangling symlinks are never replaced.

An owned, uniquely created sibling stage is written, synced and read back, then
hard-linked to the absent destination. Hard-link support is required; there is
no copy/replace fallback. Cleanup touches only the owned stage. Failure before
publication leaves no new destination. After publication, stage-cleanup or
directory-sync problems produce warnings and retain the result. Non-Unix
directory sync is unavailable through this implementation; no power-loss
guarantee is made. Exit 0 means help or publication, with any warnings relevant.
Exit 2 means error and no destination was published by that operation.

## Existing consumer route and verification

Include the resulting cooked model and applicable licensing notices in a newly
authored package/version, then use existing install, Load/Assign and save-binding
steps. The tool does not mutate installed assets, package locks, scenes or
collision. Cooked output includes decoded pixels and can load without the recipe
or original cooked input; preserve those inputs separately for reproducible
authoring.

Model tests independently construct cooked primitives and literal rebasing
oracles. They cover multi-primitive origins, nonuniform/mirrored transforms,
geometry/material preservation by float bits, signed-zero identities, f32
quantization, boundaries, source admission, exact provenance, input/output
budgets and safe CLI publication/failures. There are 26 test declarations: 25
active on Linux, 24 on macOS and 22 on Windows. A platform-inapplicable fixture
is not counted as a pass or an ignored test. The Unix socket fixture requires
socket creation; a restricted executor's failure requires explicit local
exception evidence and must not be reported as socket coverage or a full pass.
No new test is ignored.

```sh
cargo test --locked --offline --release -p orr_model --features import \
  --test static_model_pivot
cargo clippy --locked --offline --release -p orr_model --features import \
  --all-targets -- -D warnings
```

Local model-only offline validation is separate evidence and does not establish
bindings, GPU, export, native-window, Windows/macOS or physical-device acceptance.
Existing protected Room checks and required CI remain mandatory. Consumer and
export gates must execute their source-bound tests, including explicitly invoked
ignored device/export tests; a listing or ignored inventory is not acceptance.
Full hosted checks must pass for the exact reviewed head before publication is
considered verified.
