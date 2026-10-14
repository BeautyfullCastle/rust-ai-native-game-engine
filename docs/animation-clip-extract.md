# Extract a named animation clip from a GLB

This optional offline authoring tool appends one named subclip to an existing,
self-contained GLB. Use it when an imported take contains several actions, or
when only a shorter interval should loop. The output is a normal GLB accepted
by the existing animation importer. No new runtime format or public runtime API
is introduced. This is a bounded part of #100, not a complete animation editor.

```sh
cargo run --locked --release -p orr_model --features animation,import \
  --example animation_clip_extract -- \
  --input assets/animation_demo/animated.glb \
  --clip 0 --start 0.25 --end 0.75 --name short-action \
  --output /existing/trusted/directory/short-action.glb
```

The six options are required and may appear in any order. `--help` is supported
alone. Without both Cargo features, the example exits with a clear error. Time
is in seconds, parsed as finite `f32`; the range must satisfy
`0 <= start < end <= selected clip duration`. Clip selection uses its zero-based
index, avoiding ambiguous original names. The new name must be nonblank, at most
128 UTF-8 bytes, contain no control characters, and differ from every old name.

## Authored result

- All original animation indices and animation JSON are preserved. Exactly one
  new clip is appended. It begins at time zero and ends at `end - start`
- Original interior keys strictly within the selected interval are retained with
  rebased times. Sampled start/end keys preserve the existing per-channel clamp
  behavior, including late-starting, early-ending and single-key channels
- Translation/scale use the existing LINEAR or STEP semantics. Quaternion tracks
  use the existing normalized shortest-arc interpolation. Floating-point
  subdivision can introduce rounding differences; this is not bit-identical pose
  sampling across different key representations
- A STEP jump exactly at the start is already applied at new time zero. A jump
  exactly at the end appears at the final key. Loop playback follows the existing
  player, so looping at duration starts again from zero
- Original node, hierarchy, geometry, skin, material, texture, image and accessor
  indices are retained. Original BIN bytes, including padding, remain an exact
  prefix; new key accessors and buffer views point only into appended data
- Existing JSON metadata, including supported `extras`, is preserved as JSON
  values. Whitespace and object-key order are reserialized; the output is not a
  byte-for-byte source copy. Source files are never changed

Only GLB 2 with exactly one JSON chunk followed by one embedded BIN chunk is
accepted. There must be one buffer with no URI. Images, when present, must use
embedded buffer views. External files, data URIs, additional/unknown chunks and
unsupported source fields are rejected. All current importer restrictions apply,
including STEP/LINEAR TRS only: no CUBICSPLINE, morph targets, extensions, root
motion, retargeting or animation graph is added.

The original and generated GLBs both pass the unchanged production animation
importer. Generated output also cooks and reloads through `AnimatedModel` before
any output file is published. Existing source limits remain: 64 MiB file,
4 MiB GLB JSON, 32 clips, 256 channels per clip, 262,144 aggregate animation keys,
16 MiB animation data and 1,024 accessor/buffer-view entries. The tool checks
original plus appended counts, including unused original samplers, before
allocating generated keys. Non-increasing rebased keys are rejected.

## Filesystem and publication contract

Input and output parents must be existing, trusted, stable directories. Parent
traversal and symlink ancestry are rejected. Input must be a bounded regular file;
its opened handle and length/modification metadata are checked around the read.
There is no stdin, network or resource-fetch route. These portable `std` checks
are not a capability sandbox against hostile concurrent same-user/root directory
replacement. Such replacement can race an open, including substituting a FIFO.
Concurrent in-place source mutation is outside the stable-input contract.

Output must have a portable ASCII `.glb` basename of at most 100 bytes, without
reserved Windows device names, and cannot be under an `.orr` store. Existing
outputs, including source aliases and dangling symlinks, are never overwritten.
No `--force` exists. A uniquely created sibling staging file receives the complete
output, is synced and read back, then is hard-linked into the previously absent
destination. The filesystem must support hard links; there is no copy/replace
fallback. Only the owned staging file is eligible for cleanup.

Exit 2 means no destination was published by that operation. Exit 0 means help
or successful publication. After publication, cleanup/directory-sync failures
are printed as warnings and the destination is retained. Directory sync is
unavailable through this code on non-Unix platforms; warnings do not establish
hardware power-loss durability. The original failed attempts must not be hidden
by an automatic overwrite retry.

## Existing consumer path and validation boundary

Author a new animation package or immutable higher version containing the new
GLB using the existing package workflow. Install it explicitly, load the model
with the existing animation-capable editor and select the appended clip index.
The tool does not install a package, mutate its lock, update an existing scene or
expand the Room character workflow. Existing consumers compute the output's own
source/resource digests on import; hashes from the old input are not reused.

Focused tests exercise independent GLB fixtures, original-clip/non-animation
preservation, analytic clip cuts, import/cook/reload/deformation and CLI failure
preservation. Local CPU proof does not establish actual editor, rendered output,
source-hidden export, native-window or physical-GPU acceptance. Mandatory hosted
consumer/device checks and independent review remain separate gates.

```sh
cargo test --locked --offline --release -p orr_model --features animation,import \
  --test animation_clip_extract
cargo clippy --locked --offline --release -p orr_model --features animation,import \
  --all-targets -- -D warnings
```

## Design references

- [Unity 6: extracting animation clips](https://docs.unity3d.com/6000.0/Documentation/Manual/Splittinganimations.html)
  describes deriving named frame/second ranges from a continuous take
- [Khronos glTF animation semantics](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#animations)
  and [interpolation](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#appendix-c-animation-sampler-interpolation-modes)
  define channel and quaternion behavior
- [Khronos GLB layout](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#glb-file-format-specification)
  defines the header, little-endian chunks and four-byte alignment
