# Bounded skeletal animation slice (issue #100)

This optional **presentation-only** path imports real glTF 2.0 skinning and
animation, cooks a separate `orr_animated_model` version-1 asset, samples poses,
and renders the original vertices with a per-instance GPU joint palette.
`StaticModel` version 1 and its importer remain static-only; animated content
must be explicitly selected through `orr_model/animation` plus `import`.
`orr_render/animation` enables the new renderer and model runtime. No simulation
crate acquires animation, rendering, image decoding, or window dependencies.

## Explicit supported subset

The existing static importer texture/material and URI/resource restrictions
still apply. The animated path additionally retains the full node hierarchy,
including nonjoint ancestors, rest TRS, indexed skins, inverse-bind matrices,
original local mesh vertices, four influences, clip channels and source hashes.

- JOINTS_0: unsigned byte or unsigned short, not normalized
- WEIGHTS_0: float, or normalized unsigned byte/short, nonnegative with unit sum
  (integer components must sum exactly to 255/65535 before normalization)
- Translation, rotation (xyzw quaternion), and scale tracks; STEP and LINEAR
- Rotation keys: finite unit float quaternions, shortest-path normalized slerp
- Positive, nonzero scale; finite/invertible transforms and blend linear parts
- At most 64 joints per skin, 16 skins, 32 clips, 256 channels per clip,
  262,144 aggregate keys and 16 MiB aggregate key bytes
- Existing geometry/image/source limits continue; cooked input is at most 64 MiB
  and animated accessor scanning is capped at 64 MiB of aggregate elements

Node matrix fields (use TRS), CUBICSPLINE, morph targets/weights, additional influence sets, normalized integer
rotation keys, sparse/compressed/extended schemas, and mirrored/zero scales fail
explicitly. This is not a general glTF implementation. No new registry dependency
is added; the previous glTF registry network denial is not worked around.

Unknown schema fields fail closed except inert exporter metadata. Import and
cooked load validate references, cycles/multiple parents, depth, duplicate
node/property channel targets, strictly increasing nonnegative finite key times,
finite values, counts, joint ranges, normalized weights, and aggregate budgets.

## Transform and playback contract

Matrices are column-major. Node globals are `parent_global * local_TRS`.
The skin palette is `joint_global * inverse_bind`; original mesh-local vertices
are weighted by that palette and then the caller's instance placement. The
skinned mesh node's global transform is deliberately not applied a second time.
Rigid primitives use their node global transform instead.

Every sample begins with rest TRS and applies only that clip's channels. Time
clamps independently to each channel's first/last key, including single-key
tracks. `AnimationPlayer::play` selects a clip and resets time to zero. Once
playback holds the endpoint and becomes Finished; looping wraps at duration.
Seek clamps/wraps according to mode. Pause preserves time; resume continues it.
Stop clears the selected clip and returns rest. `reset_clip` retains the selected
clip but resets time, avoiding a stale clip pose. Cooked reload creates a fresh
immutable model; callers create/reset players and obtain fresh poses for it.
No sampled float state is written implicitly to deterministic gameplay.

The CPU oracle deforms every current vertex, validates the inverse transpose of
its blended linear matrix, and computes current bounds. The GPU uploads the
original vertices and applies the same blended matrix, including inverse-
transpose normal transformation. It does not upload CPU-deformed positions.
Degenerate/reversed blends reject before submission. Each draw has its own
64-matrix uniform palette, and all submitted instances share one clear/pass.
The renderer reports current world-space bounds. This correctness-first slice
performs no bind-pose culling and makes no crowd/performance claim.

## Installed-content runnable example

From the workspace root:

```sh
mkdir -p /tmp/animation-project
cargo run -p orr_package --bin orr_pkg --release --locked --offline -- \
  install /tmp/animation-project --path assets/animation_demo
cargo run -p orr_render --features animation --example skinned_strip \
  --release --locked --offline -- --project /tmp/animation-project
# Headless actual GPU render/readback, not a hardcoded geometry fallback:
cargo run -p orr_render --features animation --example skinned_strip \
  --release --locked --offline -- --project /tmp/animation-project \
  --offscreen /tmp/skinned-strip.ppm
```

The host declares the compiled `animation` capability, reads the installed,
hash-verified GLB, imports, cooks/reloads, and renders two independent instances.
Space pauses/resumes the left instance; arrows seek; 1/2 switch clips; S stops to
rest; R restarts; L restarts the selected clip in loop/once mode. Arrow keys after
Stop leave the rest pose unchanged. The right instance continues independently.
No package is implicitly installed. Missing/tampered assets are errors.

## Verification and remaining scope

```sh
cargo test -p orr_model --features animation,import --release --locked --offline
ORR_REQUIRE_GPU=1 cargo test -p orr_render --features animation \
  --test skinned_model --release --locked --offline
cargo clippy -p orr_model -p orr_render --features orr_model/animation,orr_model/import,orr_render/animation \
  --all-targets --release --locked --offline -- -D warnings
```

The original asymmetric textured strip glTF/GLB twins include two joints, mixed
weights, a transformed nonjoint ancestor, nonidentity inverse binds, a deliberately
translated mesh node, and two clips. Tests compare current poses with a CPU
oracle and actual GPU readback, including independently changing two instances.
See `tests/fixtures/animated_strip_generate.py` and its CC0 dedication.

Issue #100 remains open: this is a standalone diffuse pass owning its depth;
shared procedural depth, skinned shadows, PBR/HDR, editor clip assignment,
undo/save/reopen, animation graphs/IK/retargeting/root motion are not implemented.
Actual window interaction requires a display and is distinct from offscreen GPU
verification. Windows local execution is deferred; focused checks do not replace
mandatory remote CI or real-device validation.

Primary contract: [Khronos glTF 2.0 skins and animations](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html).
