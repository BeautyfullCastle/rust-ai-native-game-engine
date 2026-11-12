# Upgrade a referenced static package in a new Room copy

Build on Linux x86_64:

```sh
cargo build --locked -p orr_editor --features package-upgrade-copy --bin orr_room_package_upgrade
orr_room_package_upgrade --project /absolute/Room \
  --source /absolute/art-version-2 --output /absolute/UpdatedRoom \
  --acknowledge-model-changes
```

This creates a **new** closed Room project. It never changes the original
project's package lock, saved model bindings, cache, or scene. An installed direct
package must have a different exact version and the same exact dependency graph.
Every saved static binding to that package adopts the replacement asset at its
existing package-relative path. All affected GUIDs receive verified new package
and source-file hashes, even when the model bytes did not change. Entity GUIDs,
model kind, local translation/rotation/scale, other package bindings, and the
admitted gameplay Frame are preserved. Replacement content can look different;
acknowledgement is an explicit adoption of those changes, not compatibility or
license approval.

Only schema-2 Room projects without progress identity are supported. Selected
animated bindings, missing or undecodable replacement assets, changed dependency
closures, and unsupported consumer capabilities are rejected. Other packages may
contain animated bindings only when the editor is built with their existing
`room-animated-models` support. Optional Room lighting/character documents likewise
require their normal explicit build features. Candidate assets are decoded and
the complete copied Room is admitted before publication; a read-only copy plan
alone does not establish replacement compatibility.

The copy includes the declared Room scene, model sidecar, optional camera,
lighting and character documents, and a freshly reconstructed cache containing
exactly the candidate active package closure. It does **not** clone arbitrary
unlisted project files, screenshots, inactive cached revisions, user save data,
or authoring directories. Those remain in the original project. Progress-enabled
projects are rejected rather than duplicating a save identity. Declared license
and provenance files are copied as content; redistribution rights and publisher
authenticity are not verified. No PR #317 package-source-export API is required.

The new output must be an absolute normalized absent path with an existing
non-symlink parent outside both input directories. Existing directories, files,
and dangling links are never replaced. Publication uses Linux atomic no-replace
directory rename after staging and validation. This is **not** an in-place atomic
update of a lock and a model sidecar: both are assembled inside the private copy.
A failure before publication leaves the original project and final output
unchanged. A parent-directory sync failure after publication reports the output
as published with uncertain durability; preserve it and do not automatically
retry or roll it back. A successful JSON receipt identifies the output, versions,
every affected GUID's old/new hashes, and unchanged gameplay checksum.

Source paths, saved Room bytes, raw lock bytes, package identities and declared
source content are rechecked before publication. Use trusted, quiescent input and
output ancestry: these observations are not a hostile-concurrency filesystem
sandbox. The retained snapshot budget is 256 MiB for declared package payloads,
canonical package manifests and declared Room files. Decode buffers, temporary
serialization and metadata add overhead; this is not a process-memory ceiling.

Focused Linux acceptance:

```sh
cargo test --locked -p orr_editor --features package-upgrade-copy --lib package_manager::referenced_copy::tests:: -- --test-threads=1
cargo build --locked -p orr_editor --features package-upgrade-copy --bin orr_room_package_upgrade
ORR_ROOM_PACKAGE_UPGRADE_TOOL="$PWD/target/debug/orr_room_package_upgrade" \
  cargo test --locked -p orr_editor --features package-upgrade-copy --lib \
  package_manager::referenced_copy::tests::actual_cli_new_copy_success_and_collision_preserve_originals \
  -- --exact --ignored --test-threads=1
```

CPU tests compare all affected bindings, material data, original/source bytes,
reopened copied assets, gameplay checksums, stale inputs and no-replace failures.
The CLI test executes an explicitly built local tool. These are content and CPU
game-admission checks; they do not establish a GPU image, native window,
Windows/macOS support, or a standalone game export.
