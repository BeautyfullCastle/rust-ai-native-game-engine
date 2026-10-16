# Create a new Arena project

The default-off `orr_sample/project-create` tool creates one closed, versioned,
offline sprite-only starter. It closes the project-creation portion of [#104],
while the finished collect/dodge game in [#94], other templates, native targets,
installers and general distribution compliance remain separate work.

[#104]: https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/104
[#94]: https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94

## Build once, create offline

```sh
cargo build --release -p orr_sample --features project-create --bin orr_new_arena
/path/to/orr_new_arena --output /existing/parent/my-arena \
  --template arena-2d-v1 --seed my-first-arena
```

Linux is supported. The absolute output must be a new leaf under an existing,
nonsymlink parent. An existing empty directory is an error. The bounded seed is
required: 1–128 ASCII letters, digits, dots, underscores or hyphens. On the CLI,
a seed must not start with `--`, which is reserved for option names; the typed
Rust API accepts the full allowed character set. Template, seed and output
options must appear exactly once; there are no implicit defaults.
The built binary does not need its repository, assets directory or a nonempty cwd.
It does not invoke Cargo, execute package scripts, download content or launch a
runtime. No arbitrary template directory or existing-project input is accepted.

Open the generated project in a sprites-enabled editor or project-enabled runtime:

```sh
cargo build --release -p orr_editor --features sprites
cargo build --release -p orr_sample --features project --bin arena
/path/to/orr_editor --project /existing/parent/my-arena
/path/to/arena --project /existing/parent/my-arena
```

The starter contains two authored actors, existing Arena player slots 0 and 1,
positions `[-60, 0]` and `[60, 0]`, zero Score, ordinary idle/walk sprite bindings
for both actors, and saved camera follow on the first actor. This is a playable
editing starting point, not a blank-project mode, project wizard, new gameplay
system or finished collect/dodge game. Scene and sprite/follow changes are saved
separately with the existing editor controls. Export the edited result using the
[existing exporter](arena-project-export.md) and a known trusted prebuilt runtime.

## Identity and reproducibility

The seed is an authoring namespace key, not a secret or simulation random seed.
Same seed, template version and generator version intentionally produce identical
project bytes, independent of destination. Choose another seed for another entity
namespace; reusing a seed intentionally reuses its GUIDs.

GUIDs use a domain-separated SHA256 of length-prefixed template ID, generator
version and seed: the first 96 bits are shared by all template entities, followed
by the ordered 32-bit ordinal. This preserves lexical GUID order and therefore
baked entity ordering. Typed entity references, entity/comment keys, sprite
binding keys and camera follow are remapped together. Arbitrary text and package
asset identifiers/hashes are not rewritten. This is collision-resistant
namespacing, not a global uniqueness registry or authentication.

Runtime identity remains the existing Arena game/build, simulation seed 42,
60 Hz, and two human-controlled player slots. Different GUIDs do not establish
separate network compatibility, save data, progress or high-score identities.
[Player preferences](player-settings.md) intentionally remain shared under
`arena-controls-v1`; the generator does not read or copy HOME/XDG settings.

## Files, packages and notices

The ten generated files are:

- Strict schema-2 `orr.project.json`, entry scene and version-2 sprite sidecar
- Generated `README.md` containing template/tool/seed provenance
- `orr.packages.lock.json`, written only by the existing package manager
- The installed `sample-sprites` package manifest and its complete four-file
  closure: sprite document, PNG, RGBA and `LICENSE.txt`

The template bundles immutable, explicitly allowlisted existing MIT sprite bytes
at compile time. It materializes a private temporary package source and calls
`Project::install`; it never copies the fixture's installed `.orr` tree or invents
a lock/digest. Existing full-lock verification and `PreparedRuntime` admission/bake
validate the completed project before publication. Currently unused declared RGBA
bytes and the full MIT notice remain installed. The generated scene also retains
provenance and the full MIT notice in supported header comments, so those survive
the existing exporter, which deliberately excludes root README files.

No original sprite generator is executed or copied. Package/source assets are not
modified. No font/UI preset is bundled in this first template, and existing Korean
UI/font projects continue to use their independent feature and OFL notice contract.
No settings, caches, replays, captures, `.git`, editor state, writer locks, old
exports or temporary package sources are included. This is not a general licensing
or executable-dependency-notice audit.

## Transaction boundary

Creation uses a unique private sibling transaction with separate source/project
subdirectories. Writes are create-new and bounded; final payload limits are 16
files, 1 MiB per file and 2 MiB total. Output paths are UTF-8, at most 4096 bytes
and 64 components; leaves are at most 200 bytes. Dot/parent traversal, control
characters, symlink ancestors and non-directory parents are rejected.

After actual package installation, the tool validates every final file byte and
the exact file/directory closure, validates/bakes the runtime in-process, then
rechecks closure/bytes and the destination parent before publication. Linux
`renameat2(RENAME_NOREPLACE)` publishes the complete project. The same narrow
primitive is shared with export; unsupported kernels/filesystems fail closed,
without a replacing-rename fallback. Concurrent files, symlinks or empty
folders at the destination win and remain unchanged.

Errors before publication remove only the owned transaction. A successfully
published output is never rollback scratch. Crashes may leave an owned temporary
transaction; no hostile-filesystem-race sandbox or universal power-loss durability
claim is made. This slice has Linux software-GPU verification; physical native
window and Windows creation remain unverified.

## Required verification

```sh
cargo test --release -p orr_sample --features project-create,project-export \
  --lib project_create
cargo test --release -p orr_sample --features project-create --test new_arena_cli
cargo build --release -p orr_sample --features project-create,project-export \
  --bin orr_new_arena --bin arena --bin orr_export_arena

export ORR_NEW_ARENA_BIN=/absolute/target/release/orr_new_arena
export ORR_NEW_ARENA_RUNTIME=/absolute/target/release/arena
export ORR_NEW_ARENA_EXPORTER=/absolute/target/release/orr_export_arena
# Optional: broaden masking from this repository to the whole build workspace.
export ORR_EXPORT_HIDE_ROOT=/absolute/workspace
export ORR_PROJECT_CAPTURE_DIR=/tmp/new-arena-evidence
cargo test --release -p orr_editor --features sprites --test new_arena_workflow \
  generated_arena_cpu_workflow -- --ignored --exact --nocapture
cargo test --release -p orr_editor --features sprites --test new_arena_workflow \
  generated_arena_gpu_workflow -- --ignored --exact --nocapture
```

The acceptance tests require bubblewrap and already-built binaries. Missing
isolation, binaries or GPU support fail the explicit acceptance run, rather than
silently skipping. Their normal ignored status keeps plain sprites builds from
requiring external tools. Both explicit invocations above are mandatory before
merge. At this source revision, workflow wiring remains reserved to the sole
integrator; inherited green CI does not prove these ignored acceptance tests ran.
The required CI lane must check that each exact test is listed, then run it with
`--ignored --exact`. The local evidence is from those explicit invocations.
They start with the actual built generator in an empty cwd with repository and
original binaries masked, compare seeds/locks/assets, drive the real EditorApp,
save/reopen, compare runtime per-tick movement/fire/replay, then export that same
edited project, relocate it, mask the original project/runtime/workspace and
compare checksums and real compositor captures. Existing UI, settings, export,
malformed-project, default-dependency and strict Clippy gates remain required.
