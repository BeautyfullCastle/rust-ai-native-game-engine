# Offline animation clip sequence

`animation_clip_sequence` is an optional, GPU-free authoring example for one
existing cooked animated model. It appends one reusable clip:

1. Play the entire first clip A.
2. Interpolate from A's final pose to B's initial pose for an explicit positive duration.
3. Play the entire second clip B.

This is **not simultaneous-clip crossfade**, a state machine, retargeting, or a
new runtime animation format. No simulation, importer, runtime, editor schema,
or Cargo feature defaults change. Both selected clips must use LINEAR channels
and have identical `(node, property)` target sets. STEP, CUBICSPLINE, skeleton
merging, root motion, events, and graph authoring are out of scope.

## Create a sequence

Prepare a cooked source such as `source.animated.json`, then place this recipe
beside it as `action.sequence.json`:

```json
{
  "format": "orr_animation_clip_sequence",
  "version": 1,
  "source": "source.animated.json",
  "first_clip": 0,
  "second_clip": 1,
  "transition_seconds": 0.5,
  "name": "A then B"
}
```

```sh
cargo run -p orr_model --features animation,import --example animation_clip_sequence -- \
  --recipe action.sequence.json \
  --asset-id sequences/action.sequence.json \
  --output action.animated.json
```

The three CLI pairs are required exactly once. Unknown options, positional
arguments, and extra pairs fail. Exit 0 means help or successful publication;
read any durability/cleanup warnings. Exit 2 means an error. Without both
existing `animation,import` features the executable explicitly refuses to run;
it never installs packages or enables features itself. Filesystem execution is
currently Linux x86_64-only, failing closed on other platforms; the pure authoring API
is portable. The output parent must already exist, and the output must not exist.

The source path is relative to the recipe's directory, not the working directory.
It must be a normalized portable ASCII `.animated.json` path with no traversal,
empty/hidden components, device names, backslashes, URI, absolute path, or
symlink ancestry. Asset identity is a portable relative `.sequence.json` path.
Logical paths have a 240-byte limit. The output uses a portable ASCII
`.animated.json` filename of at most 100 bytes and cannot be inside a lexical
`.orr` package store. Do not use aliases/mounts to address package stores.

## Timeline and preservation

Let `dA` and `dB` be clip durations (the maximum last-channel-key time). The
appended clip spans `[0, dA + transition + dB]`, with B beginning at
`dA + transition`. Each source channel explicitly holds its first value back to
time zero and its last value through its clip's end. These endpoint keys are
inserted only when absent, including short channels and single-key clips.
Zero-duration clips are supported; positive intervals may never collapse.
The same clip index may be selected twice.

New channels are sorted by node then translation, rotation, scale, independently
of source channel order. Original keys and values are copied, and new clamp
keys use their endpoint values. Quaternion signs are retained; interpolation
uses the existing runtime's shortest-path quaternion semantics. The transition
has endpoint keys rather than sampled intermediates. Sampling is not promised
to be bit-exact with the original clip at every shifted time, due to f32 rounding.
Any nonfinite shifted time, zero-width positive transition, or collapsed or
nonincreasing shifted key interval is rejected rather than silently merged.

The source is never edited. All existing clip indices and clip contents,
nodes, skins, geometry, indices, UVs, materials, and images are preserved.
The new asset identity must differ case-insensitively from the source identity.
Only the asset identity and primitive identity prefix change, direct-input
provenance replaces dependencies, and the new clip is appended at the old clip
count. Its name must be distinct from all original clip names.

## Admission limits

- Strict object recipe: all seven fields required; unknown/duplicate fields and
  positional arrays rejected. Recipe at most 16 KiB.
- Recipe plus direct cooked source: at most 64 MiB. Output: at most 64 MiB,
  enforced during serialization.
- Name: nonblank, no control characters, at most 128 UTF-8 bytes.
- Transition: finite `(0, 60]` seconds, remaining positive when narrowed to f32.
- Total duration: at most 3600 seconds, checked before and after f32 narrowing.
- At most 32 clips, 256 channels per clip, 262144 aggregate channel keys,
  and 16 MiB aggregate animation time/value bytes, including retained clips.
- Existing production cooked-model validation and round-trip validation apply.

## Provenance, publication, and use

The output remains `orr_animated_model` version 1. Its dependencies are exactly:

- `$source`: SHA-256 of the original recipe bytes.
- The normalized recipe-relative source path: SHA-256 of direct cooked input bytes.

This proves which direct bytes were used for authoring, not runtime source
revalidation. Install the output explicitly through the existing package flow.
The animated binding's `source_hash` is the hash of the final installed output
bytes, **not** either input hash. Existing Room/Yard animated loaders parse
cooked output first and do not reopen recipe/source provenance paths. Keep
package installation separate from this command. Mesh snapshot `$source`-only
restrictions are not bypassed and are not supported by this workflow.

Reads are bounded regular-file reads. Linux x86_64 opens use nofollow/nonblocking
flags; ancestry and file identities are checked before/after reads. Publication
uses an exclusively created mode-0600 stage in the output directory, sync and
readback, then a no-replace hard link. There is no overwrite or copy fallback.
A stage is linked or removed only while its path still matches the open file's
identity. Existing files remain untouched on validation/publication failure.
Filesystem ancestry must be trusted and stable: these checks do not constitute
a sandbox against hostile same-user concurrent directory/file replacement.

The private example module exposes `sequence_clips(recipe_bytes, asset_id,
resolver)` for CPU validation. Consumer verification must execute the actual CLI,
explicitly install output, hide original inputs, and use existing Room/Yard and
editor assign/save/reopen/play paths. API tests alone do not establish widget or
GPU rendering behavior. See issue [259](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/259)
for the independent oracle and focused acceptance scope.

## Local verification (2026-10-20)

Linux x86_64 validation passed four grouped CPU tests, one explicitly invoked
real-CLI test, and two software Vulkan tests on SwiftShader Device (Subzero).
Room and Yard acceptance uses real Load/Assign/Save/Play widgets and confirms
the intended actors' asset identity, Animated kind, exact clip 2/0, Once mode
and output hash after saving and reopening. Field setup and actor selection
also use public APIs; this is not native OS input coverage.

The consumer test packages include the derived output and an unrelated static
`foreground.glb` seed needed for valid initial Room admission. Recipe and cooked
source inputs are hidden before consumption; the static seed is not a fallback
for the sequence. Renderer pixels use an independent f64 oracle and actual
wrong-order/short-transition CLI outputs at the same sample time. Editor
captures instead establish integration and per-instance deformation, not an
independent oracle over all procedural game pixels.

Selected new example/test targets passed strict Clippy. A feature-disabled CLI
build succeeded and its actual invocation refused with exit code 2. Original
test import/hex-format errors, the invalid empty Room fixture and mechanical
style-lint failures remain in the validation record.

The new real-CLI test is explicitly ignored in generic test discovery because
it requires a separately built executable. The dedicated acceptance gate must
run it with `--ignored --exact`; four helper passes do not replace that result.
The existing Linux CI job's staged gate has 13 passing Python checker tests.
Its hosted adapter requirement is llvmpipe, not the locally tested SwiftShader.
Hosted CI, shared development/main integration, other platforms, physical GPU
and workspace-wide verification are not established by these local results.
