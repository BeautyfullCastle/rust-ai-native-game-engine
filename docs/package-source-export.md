# Export installed package sources

On Linux, recover a selected active content package and its exact dependency
closure without the original authoring directories:

```sh
orr_pkg export-source /absolute/MyGame art --output /absolute/shared-art
```

The destination must be new, absolute, normalized, and outside the source
project. Its parent must already exist; symlink ancestors and existing files,
directories or dangling links are rejected. Other platforms fail closed.

The JSON success report maps each package name to a relative source directory
named by its content digest, exact version and digest. For example, install the
reported `art` directory into another project with `--path`, and each other
reported directory with `--dependency-path`:

```sh
orr_pkg install /absolute/OtherGame --path /absolute/shared-art/ART_DIGEST \
  --dependency-path /absolute/shared-art/DEPENDENCY_DIGEST
```

Use the actual digests from the report, and repeat dependency arguments as needed.
No receipt or executable is inserted into the source directories: each contains
`orr.package.json` and exactly its declared files. Existing inspector/installer
contracts remain authoritative. Pretty manifest JSON may differ from original
whitespace, but package identity, declared payload bytes, dependency versions,
capabilities and declared license/provenance files are preserved. This does not
verify redistribution rights or publisher authenticity. Undeclared content and
unrelated active/inactive objects are excluded; unrelated payloads are not
verified.

## Integrity and failure contract

The entire active lock is admitted structurally. Each selected installed package
is read once into a bounded snapshot and compared with its exact locked identity.
The lock is reread before writing. Export never changes the source project's
lock, cache, sources or writer guard. Content-only CLI export grants no compiled
capability or asset-loading authority; the consuming host must separately admit
its actual inventory. This is not a playable game/runtime export.

Limits retain package constraints: 128 packages, 4,096 files per package, 64 MiB
per file, 1 MiB manifest and 256 MiB aggregate exported payload. During rejection, summed
payload lengths can transiently approach 576 MiB: an already retained 256 MiB
closure, the next package's 256 MiB, and another 64 MiB read before that package's
budget check. Read sentinels, vector spare capacity and manifest/metadata add
further overhead; this is not a process-memory/RSS ceiling. Output paths are
at most 4,096 UTF-8 bytes and 64 components.

**Publication is not atomic.** After all validation succeeds, `create_dir`
reserves the new destination without replacing existing paths. Files are then
created exclusively and synced. A failure after reservation reports the partial
output location and leaves it intact; an interrupted process can likewise leave
partial content. No recursive cleanup, automatic reuse, or power-loss durability
is promised. Inspect/remove that exact output explicitly or choose a different
new directory before retrying. Only a successful exit/report means writing
completed; directory existence alone is not a completion signal.

Use trusted, quiescent source/output directories. Path checks and lock rereading
are observations, not a hostile-concurrency sandbox or an atomic source snapshot.
Concurrent mutation after a file was captured cannot alter those captured bytes,
but concurrent filesystem replacement is unsupported. Reads may update atimes.

## Acceptance

`cargo test --locked --release -p orr_package --test package_source_export`
exercises source-hidden diamond dependencies, deterministic output, a fresh
project reinstall with exact identity and capability-aware `Project` asset
reading/export rejection,
CLI success/error output, tampering, paths, real FIFO, size limits and concurrent
no-clobber writers. Library tests cover retained partial publication failures.
The test inventory is explicitly supplied to `Runtime`; no actual game-host
binary is exercised. These are CPU/content checks, not GPU, playable export or
other-platform proof.
