# Stormglass Windows reserved input snapshots

The opt-in `orr_package` feature `stormglass-windows-snapshot` adds
`stormglass_windows_snapshot::ReservedInputs` on Windows. The existing
content-only admission feature and audio loader remain unchanged. This adapter
reuses the audio loader's approach, rather than its private audio-specific API:
volume-GUID traversal, open ancestor handles, single-link regular files, bounded
same-handle reads and metadata rechecks. No new dependency is introduced; this
feature activates the already declared `windows-sys` dependency.

Usage:

```rust,ignore
let inputs = ReservedInputs::open(input_directory)?;
let content = inputs.admit(trusted_runtime_sha256)?;
// Consume content.runtime(), content.scene(), content.files() while inputs lives.
```

Only `coop_defense.exe` and `game.scene.yaml` are read from one existing folder.
Launcher bytes stay repository-owned. Inputs must be a non-root, absolute local
fixed NTFS drive path, at most 240 UTF-8 bytes, without device/UNC/extended paths,
streams, reserved DOS names, dot components or ambiguous trailing dots/spaces.
Every ancestor is opened without following its final reparse point and retained
using read/list access with read-only sharing. Input handles likewise permit
read sharing only. Existing write/delete handles therefore fail closed; ordinary
new writes, deletes and renames conflict until the owner is dropped. No ACL or
permissions are changed. Single-link regular files are capped at 256 MiB for the
runtime and 64 KiB for the scene, with max+1 reads and before/after metadata checks.
`admit` rechecks held handles before and after the existing exact SHA256, PE and
strict scene checks. Its borrowed result cannot outlive the reservation owner.

This is **not a publisher, executable verifier or hostile same-user sandbox**.
Pre-existing writable mappings and privileged/kernel interference are outside
the reservation guarantee. The two captures are sequential, not an atomic
folder snapshot. The runtime pin proves captured-byte integrity, not source
trust; callers must obtain it from a trusted source. After owner release, no
claim applies to the original paths. Extra files are untouched and not admitted.

Still missing: stage creation/protection and rechecks, nonexistent-output
reservation, bounded production smoke, copied-byte verification, no-replace
atomic publication and publication receipts. Nothing is executed or published,
and the exporter UI remains locked. Remote publication restrictions are unchanged.

Focused Windows checks:

```text
cargo test --offline --locked -p orr_package --features stormglass-windows-snapshot stormglass_windows
cargo clippy --offline --locked -p orr_package --all-targets --features stormglass-windows-snapshot -- -D warnings
```

Tests use synthetic non-runnable PE bytes and test-owned local directories. They
cover content parity and preservation, malformed paths, junction ancestry/leaf
rejection, hardlinks, missing/nonregular/oversize inputs, existing generous-sharing
write/delete handles, live write/delete/rename conflicts, error cleanup and late
scene mutation rejection. Junction tests need no privilege/settings changes.
These checks do not establish Linux, whole-workspace, real-runtime, smoke,
no-replace/concurrent-output, GUI or hardware coverage.
