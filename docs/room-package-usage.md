# Saved Room package usage

`room_package_usage` is an optional, read-only authoring example. Before manually
reassigning models or asking the editor to remove/update a package, it reports
**every saved entity binding** to an explicitly named installed package revision.
It does not reassign, remove, update, repair, save or start a runtime.

```sh
cargo run -p orr_sample --example room_package_usage --features room-project -- \
  --project /path/to/room --package sample-imported-scene --digest FULL_INSTALLED_DIGEST
```

Use `room-animated-models` instead for the explicitly compiled animated Room route.
Find the exact installed digest with the existing `orr_pkg explain PROJECT NAME`.
All three options are required exactly once. The digest is 64 lowercase hex digits.
`--help` explains usage. A default-feature build fails before opening a project.

Successful stdout is one JSON document with `scope: saved-room-model-bindings`,
the entry scene, existing package dependency explanation and GUID-sorted bindings.
Each binding includes its optional display name, asset, static/animated kind,
package digest and source hash. Entities sharing one asset are not collapsed.
Errors produce no success JSON and return failure. Missing/invalid packages,
wrong digest, invalid saved Room content, unsupported capabilities and damaged
installed objects fail closed. Installed source directories are not required.

## Scope and safety

Only persisted scene/model data is observed. Unsaved editor changes are excluded.
An empty binding list is **not** removal permission or a claim of removal safety:
other package dependencies and the editor's current unsaved state still matter.
Use existing editor assignment, Undo/Redo and Save, then its checked package
removal/update workflow. Partial or unsaved reassignment leaves saved references
in place, so those references continue blocking checked removal.

The query uses the real compiled Room capability inventory, full installed-byte
verification, existing dependency explanation and complete saved Room admission.
It never bypasses admission with an installation-only runtime. It checks observed
manifest/entry/lock consistency but assumes a trusted, quiescent directory. It is
not an atomic snapshot against concurrent or hostile filesystem replacement and
cannot reserve future mutation authority. No writer lock is acquired or stolen.

Implementation stays in the optional sample example and private helper; there
are no core, dependency, Cargo feature, shared schema, panel or runtime changes.
Tests include a CPU consumer of the real editor model-assignment/Save and checked
removal APIs. That consumer is not GPU, widget, native OS input or general UI
unsaved-guard acceptance. This is a new reconstruction; previous unpublished
source hashes or historical test records are not evidence for this source.

Verification is grouped with Room/package testing, reusing a build target. No
new feature-specific workflow or duplicated acceptance harness is introduced.

The same sample cases can run with `cargo test -p orr_sample --release
--features room-project --example room_package_usage` (or `room-animated-models`).
This example test harness avoids integration-test-triggered linking of unrelated
sample binaries. The integration target remains available; do not count both
executions as distinct coverage. Build the example normally to check CLI main.
