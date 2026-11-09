# Attach Room controls

After authoring a controls sidecar, explicitly attach it without hand-editing the
project manifest:

```sh
cargo build -p orr_sample --release --no-default-features --features room-controls-attach --bin orr_attach_room_controls
orr_attach_room_controls --project '/path/to/Room project' --controls custom.json
```

This first attachment tool supports Linux x86_64, plain schema-2 Room projects,
and a validated existing root-local ASCII `.json` filename of at most 100 bytes. The project must omit
`entry.controls`; null, an existing mapping (even the same mapping), progress,
lighting, and character profiles are rejected. It does not create/copy sidecars,
replace a prior mapping, change packages, or enable runtime capabilities.
Reopen with a runtime/editor supporting `room-input-actions` (main editor Play
also needs `room-input-map-play`).

Close editors and other writers first. Work in a trusted, stable local directory:
this is ordinary conflict detection, not a sandbox or compare-and-swap against
hostile simultaneous filesystem replacement.

The tool validates the complete current project and a private candidate project
using production admission. Scene checksum and exact controls bytes must match.
It preserves all unrelated manifest JSON values and every sidecar byte; manifest
whitespace/order may change. Unsupported, duplicate, malformed or oversized
metadata, traversal, symlinks, special files, and hardlinked manifest/controls are
rejected. A failed validation does not modify the project.

Before replacing the manifest, it publishes a synced byte-exact original backup
as `orr.project.before-controls.json` without replacement. An existing file,
directory or symlink at that name stops the operation. The new manifest is staged
and synced before an atomic rename; captured project inputs are rechecked before
publication. No automatic rollback or backup deletion occurs.

Success prints the backup path. If a late failure happens before rename, the
manifest remains unchanged but the backup can remain. Exit 3 means the mapping
was applied, but final directory sync failed and crash durability is uncertain.
Inspect both files before retrying or restoring a backup; never blindly repeat a
failed operation. Exit 1 means an error; read its publication-state message.

Focused automated tests cover semantic/byte preservation, production reopening,
invalid manifests, unsafe controls, backup collisions, changed inputs and injected
stage/sync/backup/rename failures. These CPU/filesystem checks do not establish
native keyboard, GPU, Windows or other platform acceptance.
