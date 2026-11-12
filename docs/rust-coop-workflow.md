# Rust web recipes and asset staging

With `orr_pkg` built on Linux using optional `coop-web-authoring`:

```sh
orr_pkg build-coop-recipe --recipe project.json --output new-game.html
orr_pkg copy-coop-web-assets --source-dir crates/orr_coop_defense_web/web --output dist/new
bash tools/build_coop_defense_web.sh dist/new-delivery
```

## Portable recipe

The existing `orr_coop_project` version 1 JSON is unchanged: required `format`, `version`, `build`; optional `scene`, `animations`, `audio`. It is limited to 8 KiB with duplicate/unknown keys and noninteger/bool versions refused. Paths are relative to the recipe, at most 1024 characters, with no absolute names, empty/dot/parent components, backslashes, colons, NUL or symlink ancestors/leaves. Inputs must be regular files or the designated build directory. CLI paths additionally follow the existing bounded native path policy.

The command consumes the same typed offline HTML hub, not a second packer. It uses the supplied build's real WASM and sampler gates as applicable, rechecks the original recipe before publication, and never executes build commands from JSON. Moving the complete relative input tree keeps the recipe usable. Output is always explicit/new and existing output is preserved. The native spelling uses `--recipe` instead of the retained Python positional argument.

## Generated build assets

Copy snapshots every required web file except Cargo-owned `game.wasm`, plus the entire optional `assets` tree, including empty directories, before any destination writes. It refreshes only corresponding regular generated files and preserves unrelated files/directories. Existing optional destination assets not in the source are not deleted. Without `--wasm`, existing `game.wasm` is untouched; the official script explicitly supplies the compiled WASM for the same safe native copy path.

All source and destination ancestry is no-follow. Source/output same-or-descendant relationships in either direction are refused. Every existing destination type is checked before the first replacement; symlinks, special files and file/directory collisions are errors. Each generated file uses an exclusive sibling, synced data and identity rechecks before replacement. Absent targets use no-replace publication; intended existing regular targets may be replaced under the trusted/quiescent-directory assumption. This is not a hostile-concurrency compare-and-swap or whole-directory atomic release.

A later filesystem failure can leave earlier files updated and retain a private sibling. Such errors describe partial progress, not rollback or permission to blindly retry. No recursive cleanup removes foreign data. Copy preserves content bytes and empty-directory structure, **not Python copytree's timestamps/mode metadata (new native files use0600 and directories0700, subject to umask)**. The source tree remains unchanged. Traversal, file counts/depth, per-file and aggregate bytes are bounded; see CLI help for exact limits.

## Official pipeline

`build_coop_defense_web.sh` now builds the native packaging tool and the actual web WASM, calls Rust asset staging with the explicit compiled WASM, then calls Rust HTML packaging. There is no Python invocation in this official path. The script refuses a preexisting `stormglass-foundry-offline.html` before Cargo or output writes: select a fresh output directory for a new completed delivery. It also refuses a symlink/nonregular `game.wasm`; the native copier checks the remaining path/type boundaries. Standalone asset refresh still intentionally overwrites corresponding regular build files.

Existing Python tools remain internal differential oracles. Node remains necessary for authored Scene/animation execution gates; no-option HTML does not claim to execute its opaque WASM or atlas. Focused tests compare old/new copied bytes and recipe payloads, moved relative inputs, rejection and output preservation, followed by an actual official Cargo→WASM→Rust copy→HTML run. Offline checker/Node evidence is not browser/GPU/physical audio or a played-game result.

See the [complete authoring migration map](rust-authoring-migration.md) for optional features, remaining Python verification/font tooling and Node conditions.
