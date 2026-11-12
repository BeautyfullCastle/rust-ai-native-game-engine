# Compare authored package content before updating

`orr_package_diff` is optional offline authoring tooling. It adds no renderer,
editor, simulation, networking or runtime capability dependency.

```sh
cargo run --release --locked -p orr_package_diff --bin orr_pkg_diff -- game art --source authored/art
cargo run --release --locked -p orr_package_diff --bin orr_pkg_diff -- game art --source authored/art --json
```

Install the original package using the existing `orr_pkg` workflow. Edit the
owned authored source and its declared manifest file list, then run this command.
It reports sorted added, removed and content-changed paths with before/after
SHA256, unchanged counts, package digests and complete before/after manifests.
Version, engine, capabilities and exact dependency requirements remain visible
when all file bytes are identical. Undeclared files are outside the comparison.
No rename or semantic asset equivalence is inferred.

Edited content at the same package version can be compared and returns exit 1.
The existing installer rejects changed contents with the same name/version; bump
the authored manifest version before separately applying the update. Comparison
does not bypass that immutable-version rule or guarantee an install will succeed.

Exit **0** means equal; **1** means successfully compared with differences;
**2** means usage, admission, verification, serialization or I/O error. `--help`
returns zero. JSON has schema 1 and the same sorted data as human output.
Paths use portable package-relative spelling; human paths are quoted and escaped.
Do not interpret exit 1 as a broken command. Before-emission failures leave stdout
empty. A sink/write/flush error can leave a partial report and returns 2; consumers
must require a successful comparison exit code and a complete parsed report.

The command never installs, removes, reclaims objects, writes locks/manifests,
executes hooks, launches tools or accesses a network. For dependency resolution
and apply feasibility use the existing separate `orr_pkg install --dry-run` flow.
Candidate inspection validates dependency names and exact-version syntax, not
availability or resolution. Required capabilities are data, not granted features.
The installation-tool API cannot be used as runtime asset-loading authorization.

## Trust and resource limits

All installed declared content is checked with `Project::verify`; the authored
candidate is inspected using `inspect_for_install`. No private object paths are
constructed by this tool. Existing schema, engine, digest, installed dependency
resolution, regular-file, no-symlink and portable-path checks remain authoritative.
Missing or corrupt content produces an error, never a metadata-only fallback.

Initial admission allows at most 8 installed packages and 8,192 declared files.
This is a preflight policy, **not a concurrent-work cap**. `verify` rereads the lock:
a concurrent valid replacement can contain up to the inherited 128 packages and
524,288 file entries. Successful installed payload reads can therefore total
32 GiB, plus at most one 64 MiB+1-byte rejection read; manifests/lock metadata are
additional. The unchanged admitted case has at most 2 GiB successful payload.
The returned and final locks must equal the initial lock or comparison fails.
There is no retry. Verification is not an atomic filesystem snapshot, writer
reservation, ABA-proof transaction or promise about a later install. Hostile
same-user/root path replacement retains the package API's documented limitations.

Each source has existing limits: 1 MiB manifest, 4,096 files, 64 MiB per file,
256 MiB aggregate payload, 240-byte ASCII paths and 16 path components. Source
inspection retains a complete snapshot, up to 256 MiB payload plus one 64 MiB
read before aggregate rejection (64 MiB+1 on file growth), plus metadata and
allocator overhead. Installed verification retains one file buffer at a time.
These describe logical payload/work, not RSS or latency guarantees. The selected
old/new path union is at most 8,192. Complete report serialization has an 8 MiB
limit before emission. Do not run this as per-frame work.

## Validation and next step

Linux-local acceptance covers actual install/edit/compare, exit statuses, sorted
hashes, metadata-only changes, unchanged inputs, corrupt installed content,
invalid paths/symlinks, resource bounds and sink failures. Run release tests for
`orr_package` and `orr_package_diff`, and strict Clippy for the diff crate.
Unchanged workspace checks may be reused under the integration contract; this
is not a fresh full-workspace or cross-platform certification. No GPU is needed.

After comparison, use the existing dry-run and saved Room package-usage checks
before a separate guarded update. Diff does not authorize or perform that update.
