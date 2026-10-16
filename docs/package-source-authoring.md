# Author a local content package

The optional Windows `package_source` example creates `orr.package.json` for an existing
asset directory. Explicitly choose its metadata and files instead of writing JSON
by hand. It uses the package manager's existing `Manifest` and
`inspect_for_install` contracts; installation, dependency resolution, immutable
objects and locks remain the responsibility of `orr_pkg` and the consuming host.

```powershell
cargo build --locked --release -p orr_package --example package_source --bin orr_pkg
New-Item -ItemType Directory .\MyModels
Copy-Item .\assets\imported_scene_demo\foreground.glb .\MyModels\
Copy-Item .\assets\imported_scene_demo\LICENSE.txt .\MyModels\
.\target\release\examples\package_source.exe --source (Resolve-Path .\MyModels).Path --name my-models --version 1.0.0 --engine '^0.0.1' --capability models --file foreground.glb --file LICENSE.txt
.\target\release\orr_pkg.exe inspect .\MyModels
New-Item -ItemType Directory .\MyGame
.\target\release\orr_pkg.exe install .\MyGame --path .\MyModels
.\target\release\orr_pkg.exe verify .\MyGame
```

Both directories must be chosen deliberately. The source directory already
exists and is supplied as an absolute disk path, not a drive root, UNC/device
path or a path with `.`/`..` components; its manifest must not exist. Canonical
Windows verbatim disk paths are also accepted. `--help` describes the options. Required
single-value options are `--source`, `--name`, `--version` and `--engine`.
Repeat `--file PORTABLE_RELATIVE_PATH` for each intended file, and optionally
`--capability NAME` and `--dependency NAME=EXACT_VERSION`. At least one file is
required by this helper. Duplicate flags or repeated values are rejected rather
than silently discarded. Capabilities and dependencies are never inferred from
extensions, source contents or folder names.

Names, exact canonical versions, engine compatibility, capabilities, dependency
metadata, portable file paths and bounds are checked against the authoritative
package inspector. Declared paths use portable ASCII with `/` separators, not
absolute paths, traversal, backslashes, case collisions or device names.
Metadata uses sorted sets/maps and deterministic pretty JSON. The same choices
produce the same manifest bytes; the source location is not serialized.

Bounds match the existing inspector for content: 4,096 files, 64 MiB per file,
256 MiB total declared bytes, 1 MiB manifest and 128 capabilities/dependencies.
The helper also bounds source paths to 4,096 UTF-8 bytes and 64 components,
versions to 128 bytes, engine requirements to 256 bytes, and CLI input to 8,500
arguments and 4 MiB of native encoded argument bytes.

Only explicitly declared regular files are copied into a bounded private
validation snapshot. Symlink/reparse parents and files are rejected before
reading; Windows opened-file metadata is checked as well. The helper observes
ordinary filesystem changes and rechecks declared source bytes before publishing.
It is not a sandbox against hostile concurrent parent-path replacement.

The fixed destination is `<source>/orr.package.json`. A synced same-directory
temporary manifest is published with `NamedTempFile::persist_noclobber`.
Existing files and directories, including a concurrent winner, are not replaced.
No crash-atomicity or power-loss durability claim is made. Only the new manifest
is retained; temporary validation data is cleaned up on normal success/failure.
Reading can update filesystem access times.

Unlisted files are not read into the package, copied, executed or installed.
Explicitly declared scripts are inert content, not hooks. The helper does not
install anything, write a project lock/object, invoke Cargo or a runtime, download
content, modify credentials, or authenticate publishers. Source content remains
mutable; normal installation recomputes its identity. No candidate digest is
presented as an installed identity.

`inspect_for_install` checks content and engine compatibility, **not** whether a
game was compiled with the declared capabilities. A manifest saying `models`
does not enable model rendering. The real consuming host must admit its actual
compiled capability inventory; the CLI does not manufacture that inventory.
Declared dependencies must be available when installation resolves the graph.

## Verification

```powershell
cargo test --locked --release -p orr_package --test package_source -- --test-threads=1
$env:ORR_PACKAGE_SOURCE_BIN = (Resolve-Path .\target\release\examples\package_source.exe).Path
$env:ORR_PKG_BIN = (Resolve-Path .\target\release\orr_pkg.exe).Path
New-Item -ItemType Directory C:\Evidence\PackageSource
$env:ORR_PACKAGE_SOURCE_EVIDENCE = 'C:\Evidence\PackageSource'
cargo test --locked --release -p orr_package --test package_source -- --ignored --exact actual_cli_creates_inspects_installs_and_reads_package --test-threads=1 --nocapture
```

The actual CLI case requires explicit existing executables and a fresh empty
evidence directory; it must not silently skip missing inputs. The ordinary test
run's ignored case is not executed CLI acceptance. Package filesystem checks do
not prove model/sprite decoding, a rendered game, editor integration or export.
Those consuming workflows require their own actual checks. The source example
adds no public engine API, schema, Cargo dependency or default capability.
Other platforms fail closed before opening a source directory; the existing
cross-platform package manager is unchanged.
