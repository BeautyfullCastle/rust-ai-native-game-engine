# Independent user-game authoring bootstrap (Linux)

## Scope and completion criteria

`tools/bootstrap_user_game.py` materializes the existing reviewed static-game
starter outside the engine repository. It does not invent a sample, game enum,
registry, player, runtime, export format, editor, or package manager. Existing
Delivery and Switchboard sources, editor/player/bundle binaries, scene includes,
and portable acceptance tests are reused byte-for-byte.

A successful creation means:

1. The local engine source matches pinned commit
   `27e53b356b25aa505995f3e8b322093948ef9db0`, tree
   `59158bede83603d7cb272ce7a101b4178956c598`, against a 2,026-file inventory whose
   Git tree identity is independently reconstructed. All dependency crate files,
   not just the starter, are verified. The inventory is a content pin; a checkout
   need not have Git metadata or claim to be the user's repository.
2. A new standalone Cargo workspace owns explicit package metadata, its sources,
   scenes, portable tests, and its future lockfile. The library retains the name
   `orr_static_starter` to preserve existing Rust imports, while `--name` names
   the user's Cargo package. Local engine dependencies retain original features,
   including `orr_remote` default-features=false and the editor static-games route.
3. Creation is staged and atomically published without replacing any existing
   destination. Symlinks, traversal, missing or changed source files, untracked
   source/configuration, and executable-mode changes fail closed.

It does **not** establish a successful dependency resolution, Rust build, native
window run, rendered output, packaged delivery, source-hidden execution, or CI.
Those are separate bounded gates. No Cargo, Git, compiler, shell, runtime, network
request, credential access, or downloaded code is executed by the bootstrap.

## Usage

Requires Linux, Python 3.11+, and libc/kernel support for `renameat2` with
`RENAME_NOREPLACE`. Unsupported platforms fail closed; no overwrite fallback is
provided. The destination's parent must already exist. Engine and destination
paths may contain spaces, but must not contain symlinks or `..` components.

```sh
python3 /path/to/tools/bootstrap_user_game.py \
  --engine /absolute/path/to/pinned-engine \
  --destination '/absolute/path/to/projects/my game' \
  --name my-game
```

Keep `user_game_bootstrap_source.json` beside the script. The tool's files belong
outside the pinned engine checkout: adding this new tool to a later engine tree
must not be confused with the exact PR288 engine input. Use a separate immutable
checkout or source archive of the pinned tree. Top-level `.git` and `target` are
excluded from source verification; other untracked files, including Python
bytecode caches and Cargo configuration, are rejected. Empty directories are not
source. The script cannot prevent another process changing the engine after
verification; keep inputs quiescent and immutable during and after creation.

The generated README contains explicit project-root editor, headless player,
concrete standalone player, native build, and trusted-runtime bundle commands.
They omit the upstream `-p orr_static_starter` selector because the generated
package has the user-supplied name. Its first dependency step
is `cargo generate-lockfile --offline`, not `--locked`: no Cargo.lock exists yet.
A cache miss is an explicit blocker, not permission to download. Review and own
the newly resolved lockfile. Subsequent portable commands select finite tests:

```sh
cargo test --offline --locked --no-default-features --features standalone,bundle \
  --lib --test standalone_contract --test bundle
cargo test --offline --locked --test acceptance
```

These commands are instructions for the later acceptance gate, not evidence that
this bootstrap ran them. Absolute engine paths mean moving the engine checkout
requires intentionally updating the manifest and provenance. The generated
workspace preserves the original release profile and clippy float warning.
The template license is MIT OR Apache-2.0; review user-project licensing.

## Includes, build scripts, and test boundaries

The starter has no `build.rs`. Its scene `include_str!` expressions refer only to
copied `scenes/` files. Engine dependencies remain in their original engine
workspace, where inherited package fields and any crate build scripts resolve
normally. The bootstrap validates those files but never executes build scripts.
Cargo dependencies and build scripts execute only in a later explicit build.

`src/bin/player.rs` remains auto-discovered alongside explicit editor, concrete
native players, and bundle binaries. Portable tests expecting
`CARGO_BIN_EXE_player` therefore keep that target. Two engine-only files are not
copied: `tests/standalone_x11.rs` and `tests/support/standalone_x11.rs`. Its explicit
test entry and `standalone-native-acceptance` feature are removed because that
source-fence harness finds the repository via `CARGO_MANIFEST_DIR/../..`.
All other Rust sources are preserved. Remaining `native_window` and
`standalone_editor_trace` tests are **not portable acceptance**: they need their
renderer/CI environment (including their documented GitHub evidence variables).
Do not advertise `cargo test --all-features` as an ordinary generated-project
check or interpret copied native tests as executed native evidence.

## User-source identity versus engine identity

`provenance.json` records the engine commit/tree, location, omitted tests, and
SHA256 of generated files. It is a creation receipt, not proof of later edits.
Its `user_source_commit` is null: the engine pin is not the user's source identity.
The user should commit generated source, manifest, provenance, and their own
Cargo.lock to their own Git repository before native delivery. Compile the
concrete player with `ORR_STATIC_STANDALONE_BUILD_SHA` set to that clean **user
commit's** lowercase 40-hex SHA; pass the same SHA to the existing bundle tool's
`--source-sha`. Do not stamp user changes with an engine-only commit. Runtime
binary/hash review and explicit trusted-runtime admission remain unchanged.

Authoring intentionally depends on engine source. A source-hidden *delivered
bundle* is a different artifact produced and tested through the existing bundle
and concrete player path. A generated folder is not such a bundle and this tool
makes no delivery certification. `UPSTREAM_README.md` preserves historical
repo-root instructions verbatim; use the generated README for relocated paths.

## Focused Python verification

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools \
  -p test_bootstrap_user_game.py -v
ORR_BOOTSTRAP_ENGINE_SOURCE=/absolute/path/to/pinned-engine \
  PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools \
  -p test_bootstrap_user_game.py -v
```

The second command enables exact-tree materialization in a separate temporary
user folder and checks unchanged Rust source bytes. Fixtures cover ownership,
metadata, lock absence, dependency changes, executable modes, missing/untracked
inputs, injected Cargo configuration, symlink ancestors/files/directories,
traversal, bad inventory, invalid names, existing destinations, publication
races, cleanup on failure, and unsupported atomic/platform behavior.
