# Attach and revise Room controls

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
implicitly replace a prior mapping, change packages, or enable runtime capabilities.
Reopen with a runtime/editor supporting `room-input-actions` (main editor Play
also needs `room-input-map-play`).

To revise controls, author a **different** sidecar, then explicitly switch from the
expected current reference with a fresh version backup:

```sh
orr_attach_room_controls --project '/path/to/Room project' --controls controls-v2.json --expect-controls custom.json --backup orr.project.before-controls-v2.json
```

Both revision flags are required together. The old reference must match exactly;
stale references, same-file switches and case-insensitive aliases of project
content are rejected. The old and new sidecars must both validate and are never
written. Backup names follow the same root-local filename rules, must not alias
project content/new controls or use reserved names, and must not already exist.
Each later revision needs a different backup name. The original first-attachment
backup remains untouched. The shared Rust API is `prepare_update(root, new_name,
expected_old, backup_name)?.commit()`; `prepare(root, name)` stays first-only.

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
as `orr.project.before-controls.json` (or the explicit revision backup) without replacement. An existing file,
directory or symlink at that name stops the operation. The new manifest is staged
and synced before an atomic rename; captured project inputs are rechecked before
publication. No automatic rollback or backup deletion occurs.

Success prints the backup path. If a late failure happens before rename, the
manifest remains unchanged but the backup can remain. Exit 3 means the mapping
was applied, but final directory sync failed and crash durability is uncertain.
Inspect both files before retrying or restoring a backup; never blindly repeat a
failed operation. Exit 1 means an error; read its publication-state message.

Focused automated tests cover semantic/byte preservation, production reopening,
invalid manifests, stale references, aliases, unchanged old/new sidecars, backup
collisions, changed inputs and injected
stage/sync/backup/rename failures. These CPU/filesystem checks do not establish
native keyboard, GPU, Windows or other platform acceptance.

## Standalone editor workflow

On Linux x86_64 build `orr_editor` with `--no-default-features --features
room-input-map-project --bin orr_room_input_map`, then run `orr_room_input_map`.
This default-off feature adds **Link saved controls to a project** only to the
standalone authoring window, not the main editor's Play panel.

1. Apply action drafts and **Save As new file** inside the project directory.
2. Enter the project directory and saved root-local controls filename. For a
   revision, select **Revise an existing controls reference** and enter the exact
   current reference and a fresh backup filename.
3. Click **Preview project link**. Production admission validates the project;
   the saved map must match the accepted authoring map. Review the project,
   old/new references and backup, then **Confirm project link** or **Cancel**.
4. Reopen the complete project in a capable editor/player. Linking does not
   change a running game, save a scene, or install the current Play map.

Pending drafts block preview. Changing any link field, accepted map or pending
map draft cancels the preview. Commit rechecks captured disk inputs and consumes
its preview even on failure. Cancel changes no project files. A durability warning
means the manifest was applied; inspect files before retrying. Map Undo does not
undo a linked manifest. Keep other writers closed throughout the operation.

Headless widget and real temporary-project tests cover preview, cancellation,
invalidated confirmation, repeated actions, attachment and revision. The
uncertain-durability message has a synthetic outcome test; filesystem fault
injection remains in the existing backend tests. No native-window result is
implied.
