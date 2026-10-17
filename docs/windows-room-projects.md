# Create and reopen Room projects in the Windows editor

The optional `windows-projects` feature joins Room starter creation, whole-project
opening and Windows game export in one editor session. It is Windows x86-64
only and remains default-off. It reuses the existing Room creator and admission
without changing simulation state/schema or the reserved export implementation.

```powershell
cargo build --release --locked -p orr_editor --features windows-projects
target/release/orr_editor.exe --game room-escape
```

The project menu is intentionally available in a local Room editor, not as an
implicit game switch from Physics, MeshNavigation or other game types. In
`File > New Room project…`, enter an absolute new directory inside an existing
parent and a deterministic ASCII seed, then choose `Create Room project`.
Existing files/directories are not overwritten. The starter includes its scene,
stable GUIDs, models/package closure, camera and local project metadata.

Creation/admission runs in a worker. A validated candidate is opened afresh,
then a new local Room host, model bindings and camera are prepared before the
current document is replaced. If preparation fails, the current editor stays
open. A created directory can remain even if subsequent admission/installation
fails: the error retains its location; do not retry by deleting it blindly.
Closing the project window does not cancel its worker.

Use the existing Room inspector/scene edits and File Save. Model and camera
sidecars have their own Apply/Save actions; File Save does not silently save
them. `File > Open Room project…` validates and opens a whole project, including
those sidecars. This is different from the existing `Open scene…` action, which
does not retarget a project's entry or model identity.

New/Open refuses unsaved scene/model/camera changes, camera drafts, proposals,
transactions, previews, active gestures, Play, capture/export work and path
dialogs. Save them, or explicitly undo/discard them first. Checks run before
worker launch and again before installation; the original scene path/checksum
must still match immediately before the swap. A project that becomes blocked
after creation is retained on disk for a later explicit Open.

Editors started with an ERP listener cannot switch projects: the existing
endpoint is not silently removed, rebound or given broader permissions. Start
a local editor without `--erp` for project creation/opening. These preflight
checks do not add compare-and-swap to the ERP protocol or isolate hostile
concurrent filesystem edits. Do not edit a project concurrently during opening.

Native GPU target registrations are reused while project-owned editor,
selection, model/camera controls and export trust/settings are reset. Rendering
uses the newly admitted project's current bindings, not old imported models.
Play/Pause/Step/Stop uses the normal document-copy play session. Saved edits
remain independent of Play.

Finally choose `File > Export Windows game…`; supply known trusted,
already-built Room runtime code and an explicit expected hash/trust decision.
See [Windows game export](windows-game-export.md) for bundle contents, guarded
publication, runtime trust, native launcher limitations and standalone usage.
Project creation does not compile a runtime, sign a release or install system
dependencies. This bounded Room workflow does not establish complete engine,
package ecosystem, MeshNavigation export or all-platform production readiness.
