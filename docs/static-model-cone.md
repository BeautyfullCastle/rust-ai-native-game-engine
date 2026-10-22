# Optional offline capped-cone authoring

`static_model_cone` creates an ordinary StaticModel v1 cooked `.orrmodel` file.
It is a private offline example under existing `orr_model/import`, not a new
runtime primitive, collider, core API, schema, feature or dependency. It does
not modify or install packages, and does not take over the reserved editor
model/box or package-manager panels.

```sh
cargo run --locked -p orr_model --features import --example static_model_cone -- \
  --recipe key.cone.json --asset-id props/key.cone.json --output key.orrmodel
```

The input already exists; the output must be new and its parent must exist.
Help/publication exits 0 (read any durability warnings); errors exit 2. Without
`import`, even help exits 2. No force, overwrite, install, stdin or network
option exists. Use the existing package install and model Preview/Use workflows
for a generated `.orrmodel`; this command itself never changes a project.

## Strict recipe and provenance

```json
{"format":"orr_static_model_cone","version":1,"radius":0.5,"height":1.0,"segments":32}
```

Exactly these five fields are required in an object of at most 16 KiB. Unknown,
missing, duplicate or incorrectly typed fields, arrays and trailing JSON are
rejected. Dimensions are finite JSON numbers in `[0.001,1000000]`, checked in
f64 before narrowing. Segments is an integer in `{8,16,32,64,128}`. Frustums,
smooth shading, arbitrary subdivisions, textures and collision generation are
not supported by this recipe.

The logical identity is a relative portable ASCII `.cone.json` path, at most
240 bytes, without hidden, empty, traversal or reserved-device segments. It
does not redirect the physical input. The sole `$source` dependency records
the SHA-256 of the exact input bytes, including whitespace. Primitive IDs are
that identity plus `#node=0/mesh=0/primitive=0` and `1`.

## Geometry contract

The cone is centered on the Y axis: apex `(0,+height/2,0)`, base circle at
`y=-height/2`, radius in X/Z. There are exactly two identity-transform primitives
and two default-white material slots, sharing a single white pixel:

1. Slot 0: N faceted side triangles, 3N independent vertices, 3N indices.
2. Slot 1: flat bottom fan, N+1 vertices, 3N indices, normal `(0,-1,0)`.

The ring starts at +X and advances toward +Z. Side triangle i is
`[bottom_i, apex, bottom_(i+1)]`; its vertices all carry the normalized outward
cross product computed from the actual f32 positions in f64, then narrowed to
f32. Nonfinite or collapsed triangles/normals are errors, not repaired geometry.
There is no degenerate top cap or zero-area apex ring. Cap triangles are
`[center, ring_i, ring_(i+1)]`, facing -Y. The position-welded surface is closed.

Side UVs per triangle are `(i/N,1)`, `((i+0.5)/N,0)`, `((i+1)/N,1)`.
Duplicated side endpoints at the +X seam are position-bit-identical with U=0
and U=1. Each face owns its apex and normal; side shading is deliberately not
smooth. Cap UVs are the X/Z projection into `[0,1]^2`, with center `(0.5,0.5)`.
Cap vertices are separate to preserve the hard bottom normal.

The ring uses the fixed 128-point binary32 angular table inherited privately
from cylinder authoring. All allowed segment counts subdivide it exactly;
cardinal zeros are positive zero. This is presentation geometry, never
simulation math. Normal square roots use f64 arithmetic. Stable ordering and
same-process byte stability are tested, not a claim of untested cross-platform
serializer or GPU equivalence. Every result is validated, cooked, reloaded and
compared semantically with exact floating-point sign bits before publication.

## Filesystem boundary

Input and output ancestry must be trusted, stable, local and link-free. Input
must be a bounded regular file; metadata identity/length changes during open
or read are rejected. Parent traversal, symlinks, nonlocal Windows namespaces,
reserved names, and output anywhere under a lexical `.orr` segment are refused.
Output names are portable ASCII `.orrmodel` filenames of at most 100 bytes.

A privately owned create-new stage is written, synced and read back, then
published with a no-replace hard link. A second write never overwrites an
existing destination. Unsupported hard links fail closed. Stage cleanup and
directory-sync failures after publication are warnings, not claims that no
output exists. Portable ancestry checks are not a sandbox against hostile
concurrent directory replacement or alternate aliases into a package store.

## Verification

```sh
cargo test --locked -p orr_model --features import --test static_model_cone_cli
cargo build --locked -p orr_model --features import --example static_model_cone
ORR_STATIC_MODEL_CONE_BIN="$(pwd)/target/debug/examples/static_model_cone" \
  cargo test --locked -p orr_model --features import --test static_model_cone_cli \
  actual_cli_publishes_validated_output_and_rejects_second_write -- --ignored --exact
```

The CPU suite uses an independent f64 geometry oracle, positional manifold and
Euler checks, positive polygonal cone volume, all five segment counts and
extreme dimensions. It also covers strict parsing, provenance, round trips,
UV seams, faceted normals, input/output refusal and no-clobber publication.
The explicit ignored CLI test requires a freshly built source-bound binary;
ordinary passing CPU tests do not imply that this test ran. Editor consumption,
actual GPU rendering, native-window interactions, hosted exact-head required CI
and development integration remain separate evidence categories.
