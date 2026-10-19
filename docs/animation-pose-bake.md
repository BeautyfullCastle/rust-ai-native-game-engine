# Bake an animation pose into a static model

This optional offline authoring example freezes one explicit sample of a cooked
animated model into the existing static model v1 format. It adds no runtime API,
model schema, editor action, package installation, or simulation behavior.

Create a `poses/frozen.pose.json` recipe:

```json
{"format":"orr_animation_pose_bake","version":1,"source":"inputs/character.animated.json","clip":0,"time":0.5}
```

The direct source is a cooked `orr_animated_model` v1 file, resolved relative to
the recipe's parent directory. It is not a GLB or a package reference. Historical
dependencies are provenance only and are never fetched.

```sh
cargo run --locked -p orr_model --features animation,import \
  --example animation_pose_bake -- \
  --recipe poses/frozen.pose.json \
  --asset-id poses/frozen.pose.json \
  --output /existing/trusted/directory/frozen.model.json
```

All three option pairs are required, unique, and order-independent. `--help` or
`-h` works alone. Without both features the executable exits 2. There is no force,
stdin, network, import, install, or overwrite option.

## Sampling and geometry contract

- Recipe objects have exactly the five required fields above. Unknown, missing,
  duplicate, null, and positional-array forms fail
- `clip` is a zero-based unsigned integer. Names are not accepted
- `time` is finite nonnegative seconds, parsed as `f64` and checked against the
  selected clip's duration before narrowing to the existing `f32` sampler. An
  out-of-range value that would round back onto the endpoint still fails.
  Positive values that narrow to zero fail; other admitted values use normal
  `f32` rounding. Negative zero samples zero
- The whole source is admitted before sampling, including unused clips, nodes,
  skins, primitives, materials, and images. Unsupported source fields fail
- Sampling uses existing STEP/LINEAR TRS semantics, shortest-arc normalized
  quaternion interpolation, and independent per-track endpoint clamping. Late,
  early, and single-key channels retain those semantics. Exact final keys are
  sampled without loop wrapping. Missing tracks retain rest values
- Weighted skinning uses the complete hierarchy, including non-joint ancestors,
  and each joint's inverse bind matrix. A skinned mesh-node transform is not
  applied a second time. Rigid primitives use their sampled global node matrix
- Output positions and inverse-transpose-normalized normals are in model space.
  Every primitive transform is identity. Entity/world placement is not baked
- UV float bits, indices and ordering, material slots, material float bits,
  samplers, image dimensions, and image bytes are preserved. Positions/normals,
  asset and primitive identity, and provenance intentionally change
- The recipe asset ID replaces only the source asset prefix of primitive IDs.
  Dependencies become exactly `$source` (SHA-256 of the exact recipe bytes) and
  the direct cooked source path (SHA-256 of its exact bytes)
- Singular or mirrored sampled transforms/blends fail. No rest-pose fallback,
  dropped primitive, approximated skin, or normal fallback is produced
- Static-model validation, cooking, reload, and bit-sensitive comparison complete
  before publication. The static local-position bound of 1,000,000 still applies;
  some positions permitted by animated model-space bounds cannot be baked

Current importer/runtime restrictions still apply. This tool does not add morph
animation, CUBICSPLINE, root-motion extraction, retargeting, animation graphs,
mesh editing, or new material channels.

## Bounds and safe publication

Recipe input is at most 16 KiB. Recipe plus direct source bytes are at most
64 MiB. Existing animated and static collection, decoded-data, hierarchy, vertex,
index, image, transform, and cooked-output bounds remain enforced.

Logical IDs are normalized relative portable ASCII paths of at most 240 bytes.
The source ends in `.animated.json`, recipe identity in `.pose.json`, and output
basename in `.model.json`. Hidden, traversal, reserved device, nonportable and
wrong-suffix paths fail. Output basenames are at most 100 bytes; `.orr` stores
are rejected case-insensitively.

Input and output parents must exist and have trusted, stable, link-free ancestry.
Inputs must be bounded regular files. Handle metadata is checked around bounded
reads. These portable checks are not a capability sandbox against hostile
same-user/root directory replacement or concurrent input mutation.

Existing outputs, including hard-link aliases and dangling symlinks, are never
overwritten. A unique sibling staging file is written, synced, read back, then
hard-linked to the previously absent destination. Hard-link support is required;
there is no replacement/copy fallback. Cleanup touches only the owned staging
file. Exit 2 means this operation did not publish; exit 0 means help or published
output. Post-publication cleanup/directory-sync warnings retain the destination;
non-Unix directory sync is unavailable. A warning is not a rollback.

## Verification

The additive `Animation pose bake` workflow runs full CPU acceptance on Linux and
Windows and required software-Vulkan acceptance on Linux. It retains source and
producer identities, exact test inventories, logs, generated cooked output and
GPU captures. The GPU test invokes the actual freshly built CLI and compares its
output against independently constructed weighted geometry and normal lighting,
with corrupt controls. A missing adapter or producer is a failure in that lane.

CPU tests independently exercise weighted skinning, non-joint ancestors, inverse
binds, nonidentity mesh nodes, rigid primitives, nonuniform normal transforms,
STEP boundaries, clamped/single-key tracks, multiple clips, exact-byte provenance,
full source admission, strict recipe/path/size bounds and no-overwrite behavior.
Unix socket refusal fixtures remain mandatory on supported hosted runners.
Restricted local environments may prohibit constructing the socket itself; that
is a blocked gate, not a passed test and not grounds for removing the fixture.

Existing repository CI remains unchanged and mandatory. Focused local debug
checks do not certify full hosted release acceptance or the original workspace
checks. Raw local logs and environment-specific paths are not part of this guide.
