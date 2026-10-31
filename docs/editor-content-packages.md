# Editor content packages

The optional, default-off manager supports saved local Room projects in Edit
mode. Build with `--features package-manager` (or `windows-projects` for the
Windows Room workflow), then choose **File > Content packages…**. The editor
supplies the admitted project root; this panel cannot redirect operations.
Save scene/model/camera edits, finish project/capture work and close export
windows first. Package workers block project switching and opening export.

## Verify and inspect saved usage

**Verify installed packages** checks installed bytes against the active lock,
then displays each package's version, exact digest, direct/dependency status
and capabilities. Metadata alone is not byte verification.

Choose **Select usage {package}**, then **Inspect saved usage** to list every
saved Room model binding for that exact verified package/digest. Rows appear in
GUID order with entity name (or unnamed), asset, static/animated kind, package
digest and source hash. Inspection runs off the UI thread using the same saved
query as `room_package_usage`; it starts no subprocess or Cargo command.

This is a read-only saved-state observation: unsaved edits are excluded. It
changes no scene, selection, history, model binding, source or cache bytes.
**Zero bindings is not removal permission or proof of safety.** Removal/update
still require their separate preview and checked commit. The report is not an
atomic snapshot or protection against hostile concurrent filesystem edits.
Verify/other package operations, source/project changes, dirty or modal blockers
and closing the panel clear the report; stale worker completions are discarded.
The entire installed lock must fit the editor's own compiled Room capabilities:
animation requires explicit `room-animated-models`, even if dependencies have
animation enabled through another feature.

## Add and use models

**Preview package addition** shows observed before/after lock changes without
activating content or reserving a permanent lock. **Add previewed package**
requires a current project/source preview and revalidates it while applying.
Only one new direct package is added; dependencies must already resolve from
the installed lock. Existing selections and identities stay unchanged. New
packages must declare `LICENSE.txt`; its presence is not legal trust.

After verification, **Preview static model** inspects static assets without entity
selection or assignment. **Use model {package}/{asset}** assigns an installed
`.glb`, `.gltf` or `.orrmodel` to the selected persistent entity without changing
its simulation transform. With `room-animated-models`, animation packages offer
animated inspection and **Use animated model {package}/{asset}** for GLB/glTF.
Assignment starts with clip zero/Loop; choose another clip or Once in the model
inspector and assign again. See [Room animated models](room-animated-models.md).

Existing binding Undo/Redo and Save remain authoritative. File Save does not
silently save model sidecars; reopen to re-admit scene, packages, bindings and
camera together. No preview activates runtime content or promises permanence.

## Update or remove a direct selection

For **Preview update**, supply an installed direct package's same name and a
different exact version. Review old/new identities, then explicitly **Update
previewed package**. Saved Room references must permit the change; other
selections and dependencies remain unchanged. Verify again after completion.

After verification, choose **Select removal {package}** and **Preview package
removal**. Review deactivated packages and retained dependencies. A selection
needed by another package is labeled **Remains active as a dependency**.
Only **Remove previewed package** commits. Saved scene and every static/animated
binding are re-admitted; referenced packages and dependencies cannot be
deactivated. Changed project/lock bytes invalidate previews.

Removal retains cache objects, original sources, scene/model/camera bytes and
history. Successful removal refreshes verified inventory. During removal,
restoration or update commits, authoring and playback input—including keyboard
and accessibility actions—is frozen through the completion frame. External
host/ERP writers are not isolated. Another project's completion never replaces
the current inventory.

## Restore the last successful removal

One in-memory receipt is retained only after successful removal and Room
post-validation. **Preview restoration**, then **Restore last removal** restores
that exact prior lock and dependency closure from verified cache, even when
original sources are unavailable. This is not arbitrary rollback, a cache
browser or persistent Undo.

Project/scene/bindings/camera/current-lock identities must still match. Missing
or tampered cache bytes are refused. Package changes, fresh verification,
project switching, authoring changes, restoration failure/success or editor
closure consume the receipt. Uncertain publication never triggers automatic
rollback; verify again. Failed publication can leave unreachable immutable
objects: do not delete them or steal writer locks automatically.

## Acceptance and limits

Saved-usage tests cover exact shared-query output, ordered complete rows,
identity/stale/feature rejection, read-only live editor state and empty-report
non-authorization. Existing removal/restoration acceptance uses owned fixtures
and synthetic egui input, including save/reopen/Play and retained source/cache
bytes. CPU or software-Vulkan evidence does not establish physical-input,
Windows or hardware-GPU coverage. No package scripts, code plugins, dependency
downloads or all-game package readiness are provided.
