# Author a local content package

On **Linux and Windows**, `orr_pkg_source` creates `orr.package.json` for an
existing asset directory using the package manager's existing `Manifest` and
`inspect_for_install` contracts. The `package_source` Cargo example remains a
compatibility entry point to the same implementation. Other platforms fail
closed before opening a source directory, including for `--help`.

## Usage

Linux:

```sh
cargo build --locked --release -p orr_package --bins --example package_source
mkdir MyModels MyGame
cp assets/imported_scene_demo/foreground.glb assets/imported_scene_demo/LICENSE.txt MyModels/
target/release/orr_pkg_source --source "$PWD/MyModels" --name my-models \
  --version 1.0.0 --engine '^0.0.1' --capability models \
  --file foreground.glb --file LICENSE.txt
target/release/orr_pkg inspect MyModels
target/release/orr_pkg install MyGame --dry-run --path MyModels
target/release/orr_pkg install MyGame --path MyModels
target/release/orr_pkg verify MyGame
```

Windows PowerShell uses the same options:

```powershell
.\target\release\orr_pkg_source.exe --source (Resolve-Path .\MyModels).Path --name my-models --version 1.0.0 --engine '^0.0.1' --capability models --file foreground.glb --file LICENSE.txt
```

Choose both directories deliberately. The source must already exist, its
manifest must not exist, and its Unicode path must be absolute and non-root.
Linux requires normalized spelling: no repeated/trailing `/` or `.`/`..`
components. Windows accepts disk and verbatim disk paths, not UNC/device paths
or traversal. All source ancestors must be real directories, not symlinks or
Windows reparse points.

`--help` lists options. Supply `--source`, `--name`, `--version`, `--engine`, and
at least one `--file PORTABLE_RELATIVE_PATH`. Repeat `--file`, `--capability NAME`
and `--dependency NAME=EXACT_VERSION` as needed. Duplicate options/values,
noncanonical versions, incompatible engines, invalid metadata and colliding
paths fail without publication. Files use portable ASCII `/`-separated paths;
absolute paths, traversal, backslashes and reserved device names are rejected.
Capabilities and dependencies are never inferred from filenames or contents.

Sorted sets/maps and pretty JSON make manifest bytes and inspected content
identity independent of source location and option order. Only declared files
contribute; unlisted files are untouched. Dependency sources must be available
when `orr_pkg` resolves the graph; use `--dependency-path SOURCE` during preview
and installation. Authoring does not install packages or edit project locks.

## Bounds and filesystem contract

Limits: 4,096 files, 64 MiB each, 256 MiB total declared content, 1 MiB manifest,
128 capabilities/dependencies, 4,096 source-path bytes, 64 source components,
128-byte versions, 256-byte engine requirements, 8,500 CLI arguments and 4 MiB
of native argument bytes. Bounded reads admit only regular files: declared
symlinks, directories, FIFOs and sockets are rejected.

Use **trusted, quiescent source and temporary directories**. Each source
ancestor and declared component is checked with
[Rust `symlink_metadata`](https://doc.rust-lang.org/std/fs/fn.symlink_metadata.html);
this does not freeze parent-directory resolution. Linux additionally compares
opened-file device/inode identity, while Windows retains reparse-aware opening.
The helper validates a private snapshot with the authoritative inspector,
rechecks declared source bytes, then publishes. These are observation checks,
not a race-proof sandbox: concurrent hostile path replacement is unsupported.
Reading may update access times.

A synced same-directory temporary manifest is published using
[`NamedTempFile::persist_noclobber`](https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html#method.persist_noclobber).
Existing files, directories, dangling symlinks and concurrent winners are not
overwritten. The API does not guarantee atomic publication on every platform;
a failure/interruption can leave temporary names. No crash/power-loss durability
promise is made. Normal cleanup removes private validation data.

Scripts are inert content, never hooks. There are no downloads, Cargo edits,
credential changes or publisher authentication. Source content remains mutable;
installation recomputes identity. Manifest capabilities do **not** enable engine
features: the consuming host must supply its actual compiled inventory and
reject unsupported content. CLI verification checks content/engine contracts,
not rendering or compiled capability support.

## Acceptance

```sh
cargo test --locked --release -p orr_package --test package_source_linux -- --test-threads=1
```

The Linux suite runs real CLI help/create/inspect, dependency dry-run/install,
source-hidden verification and explicit consuming-inventory admission/rejection.
It covers deterministic identities, preservation on invalid inputs, existing
manifests, symlink/nonregular rejection, size bounds and concurrent creators.
At the user's request, primary special-file acceptance uses a real FIFO
(`mkfifo` required), exercising the same generic nonregular-file rejection as a
socket. Symlink and directory cases remain required. AF_UNIX socket creation
returned `EPERM` in this environment: socket-specific coverage is **unverified**,
not passed. That retained test is an explicitly ignored supplemental lane; on a
permitted host, opt in with:

```sh
cargo test --locked --release -p orr_package --test package_source_linux socket_source_is_rejected_without_changes -- --ignored --exact
```

This does not change sandbox/security settings or weaken production rejection.
Windows retains `package_source_cli` and `package_source` tests; its ignored
external-executable acceptance still requires
`ORR_PACKAGE_SOURCE_BIN`, `ORR_PKG_BIN` and a fresh empty
`ORR_PACKAGE_SOURCE_EVIDENCE` directory.

These checks do not prove asset decoding, GUI placement, rendered gameplay or
export. See [mesh navigation authoring](mesh-navigation-authoring.md) for model
cooking and downstream checks. No public engine API, schema, dependency or
default capability changes are introduced.
