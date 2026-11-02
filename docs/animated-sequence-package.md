# From authored sequence to installed animated model

This optional offline CLI connects `animation_clip_sequence` output to the
existing package and mixed-model binding loaders. It does not add an animation
format, decoder, runtime, auto-installation, or enabled-by-default feature.

Build the trusted local native authoring programs:

```sh
cargo build --locked --release -p orr_model --features animation,import --example animation_clip_sequence
cargo build --locked --release -p orr_package --bin orr_pkg_source --bin orr_pkg
```

Prepare the existing [sequence recipe](animation-clip-sequence.md) beside its
cooked animated source. Then:

```sh
python3 tools/create_animated_sequence_package.py \
  --recipe actions/action.sequence.json --asset-id sequences/action.sequence.json \
  --license LICENSE.txt --name action-sequence --version 1.0.0 --engine '^0.0.1' \
  --sequencer target/release/examples/animation_clip_sequence \
  --package-source target/release/orr_pkg_source --output authored-action
```

The new package contains `model.animated.json`, `LICENSE.txt`, and
`orr.package.json`, with explicit `models` and `animation` capabilities. Install
using the normal package manager or `orr_pkg install PROJECT --path authored-action`.
The receiving editor/runtime must compile support for those capabilities;
this tool does not activate it. The appended clip is selectable through existing
animated model bindings. Package hashes verify byte identity, not distribution
rights or visual correctness.

## Deliberately unchanged semantics

The native sequencer admits the cooked source and retains its rig, primitives,
materials, images, and original clips, then appends the existing supported
A → positive-duration endpoint transition → B sequence. Its matching-LINEAR-
channel restrictions and strict resource bounds remain in force. This is not
simultaneous crossfade, retargeting, a state machine, root motion or event authoring.
Historical source references and hashes are provenance; the cooked model embeds
the content needed after source directories are removed.

The existing `orr_pkg_source` alone admits metadata and creates the manifest.
The merged [static-package helper](assembled-model-package.md) supplies the
unchanged path checks, native process timeout and exclusive publisher. Recipe
snapshot/recheck is bounded to 16 KiB, license to 1 MiB, model output to 64 MiB;
the native tools retain any stricter limits. Unknown/invalid recipes and
incompatible clips fail before final publication.

License is nonempty UTF-8 text supplied by the user, copied exactly; no ownership
or legal-rights verification is performed. Use explicit trusted local binaries
and stable, link-free, trusted/quiescent input/output directories. Native programs
run without a shell or PATH lookup and have a 60-second timeout, but are not
sandboxed. The output parent must exist; an existing destination is never
replaced. Intermediate work is private. Handled publication errors clean only
known newly created files; cleanup failure is reported. This is not crash-atomic
publication or protection against a hostile concurrent directory owner.

## Verification

Python tests cover this frontend's wiring and failure contracts, with mocked
shape-only native output clearly distinguished from valid animation evidence.
The dedicated Rust integration test runs the real frontend/native tools,
installs into an isolated project, hides input/package-source directories,
loads the animated asset, verifies rigs/clips/channels and sampled poses, and
saves/reopens an exact-digest animated Binding.

```sh
ORR_SEQUENCE_BIN="$PWD/target/release/examples/animation_clip_sequence" \
ORR_PACKAGE_SOURCE_BIN="$PWD/target/release/orr_pkg_source" \
cargo test --locked --release -p orr_model_bindings --features animated-models \
  --test animated_sequence_package_cli -- --ignored
```

This test is explicitly ignored in ordinary runs because native programs must
be built first. Successful CPU installation/loading and pose sampling do not
certify renderer, GPU, editor interaction or visual animation quality.
