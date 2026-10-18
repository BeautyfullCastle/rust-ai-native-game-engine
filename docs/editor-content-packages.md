# Editor content packages

The optional, default-off package manager is initially available only for local
Room projects in Edit mode. The editor supplies the current saved/admitted Room
project root; users cannot redirect package operations to another project in
this panel.

Build with `--features package-manager` (or `windows-projects` for the Windows
Room workflow), then choose **File > Content packages…**. Save the scene and
model/camera sidecars first; finish project/capture work and close the export
window. Package workers block project switching and opening the export window.

**Verify installed packages** checks installed bytes against the project's
active lock. A metadata listing alone is not a byte-integrity result. The panel
shows each locked package's version, digest, direct/dependency status and
capabilities after verification.

**Preview package addition** inspects a local package and shows the observed
before/after lock changes. It does not add or activate content and does not
reserve a permanent lock. **Add previewed package** requires a preview for the
current project and source path; the backend revalidates it while applying. The
addition operation adds a new package only; it does not update an existing
package identity. Removal uses the separate explicit preview below.
Only one new direct package is added at a time; its dependencies must already
resolve from the installed lock. Existing selections, manifests, digests and
file identities stay unchanged. New packages must declare `LICENSE.txt`.

Package content remains subject to the consuming Room host's compiled
capability admission. The package workflow is offline and does not execute
package scripts, start a runtime, or invoke Cargo. The presence of a license or
notice file is not evidence of legal trust. Byte verification does not decode
arbitrary content as a model. After verification, the panel offers `Use model {package}/{asset}`
for installed `.glb`, `.gltf` and `.orrmodel` files. Choose an entity before
using one; saving the resulting model binding remains a separate editor action.
With the explicit `room-animated-models` feature, packages declaring `animation`
also offer `Use animated model {package}/{asset}` for GLB/glTF assets. This loads
and assigns clip zero with Loop playback through the existing model inspector.
Choose another clip or Once playback there and assign again before saving.
See [Room animated models](room-animated-models.md) for consuming-route limits.
No preview is a runtime activation or permanence promise.

Use loads through the existing static-model admission and assigns to the
selected persistent entity without changing its simulation transform. Existing
model-binding Undo/Redo and Save actions remain authoritative; File Save does
not silently save model sidecars. Reopen the project to re-admit its saved
scene, package lock, model bindings and camera together.

Plans are observations, not durable apply tokens. Source/lock changes are
rechecked under the existing package writer guard. Unreachable immutable objects
can remain after a failed publication attempt; do not delete objects or steal
writer locks automatically. Post-publication validation failures explicitly
retain the installed lock and require fresh verification. Filesystem checks do
not isolate hostile concurrent edits. No code-package/plugin installation,
dependency downloads, update/removal or all-game package readiness is claimed.

## Remove a direct package selection

After **Verify installed packages**, choose **Select removal {package}** for a
verified direct selection, then **Preview package removal**. Review the packages
that will be deactivated and those retained as dependencies. A removed direct
selection can remain active when another selected package requires it; the panel
labels this **Remains active as a dependency**. Only **Remove previewed package**
commits the preview. Preview itself changes no project files.

Removal re-admits the saved Room scene and every static and animated model
binding. It refuses to deactivate any referenced package, including a removed
selection's dependencies. Scene, model and camera edits must first be saved.
Changed project files or package locks invalidate an earlier preview; preview
again. Project changes discard prior inspection and preview state, and package
workers block project switching. While a removal commit is running, the editor
pauses all user authoring and playback input, including keyboard and accessibility
actions, through the completion frame. External host/ERP writers are not isolated.
Worker completion never installs another
project's inventory into the current panel.

This deactivates a lock selection, not a file-deletion operation. Package cache
objects, original source directories, scene/model/camera files and document
history are preserved. Successful removal refreshes the verified inventory.
No package code is executed, and existing rights acknowledgements are unchanged.
The opt-in `package_removal_ui` acceptance uses owned Room fixtures and synthetic
egui input; its receipt and screenshots distinguish executed Vulkan validation
from physical-input or hardware-GPU claims.
