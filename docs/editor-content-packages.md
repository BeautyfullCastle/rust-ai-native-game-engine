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
initial operation adds a new package only; it does not update or remove an
existing package identity.
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
