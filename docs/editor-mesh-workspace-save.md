# Save an applied mesh workspace

With `mesh-navigation` enabled, the production editor offers **File → Save mesh workspace** for a saved, admitted local MeshNavigation scene with a placed agent. It saves the scene and already-applied changes to agent art, props, camera, lighting, HUD, and, when `mesh-animated-models` is enabled, animation. Existing **Save / Ctrl+S** still saves only the scene.

## Workflow

1. Create or open a mesh scene, place the mesh agent, and save it to a file.
2. Edit the desired presentation panels. Choose each panel's explicit **Apply** or **Update selected prop** action; selecting a source or typing a field alone does not apply it.
3. Choose **File → Save mesh workspace** once. Clean optional presentations are left absent; saving does not create six default sidecars.
4. Reopen the saved scene, play it, duplicate its workspace, or export it through the existing mesh workflow. Mission settings remain embedded in the scene, not in a seventh presentation sidecar.

The command is disabled while an unapplied form, geometry/agent draft, viewport gesture, pending load, invalid or stale presentation, open proposal, path dialog, capture, package operation, duplicate, export, or new-game operation requires attention. It requires an idle local Edit host without an ERP listener. Resolve the stated blocker explicitly; Save never applies, repairs, reloads, removes, or discards a draft for you. An animation sidecar also remains blocked in a build without animation support.

## Persistence and failures

Preflight checks every participating panel before the first write. After that, the scene is saved first, followed by dirty applied sidecars through their existing scene/graph-bound, digest-guarded stores. Clean panels are no-ops. Saving presentation data does not add a simulation undo entry or change the admitted mesh graph.

This is a **sequential save, not a multi-file atomic transaction**. A filesystem or external-writer conflict can still occur after preflight. The command stops at the first failed write and names the portions already saved; earlier writes remain on disk, while the failed and not-yet-saved applied changes stay pending. Existing conflict checks preserve externally changed sidecar bytes. Fix the conflict deliberately and retry; no rollback or hostile/concurrent-writer isolation is promised.

The six sidecars remain the existing optional scene-local formats. This command adds no public engine API, runtime schema/profile, dependency, package manager, or exporter format. The editor's mesh feature remains optional; saving does not install packages or copy their source directories.

## Acceptance scope

Focused CPU regressions cover clean no-op saves, unapplied-state refusal, scene persistence, optional-file absence, invalid presentation preservation, Edit/Play guards, applied sidecar writes, and external conflicts. The existing composed production-App acceptance authors six changed presentations, exercises the File action, and continues through duplication, fresh reopen with original workspace/package roots hidden, mission Play/replay, and export. Versioned local Windows receipts report actual executions separately; documenting this contract does not claim that an unexecuted run passed.

See [workspace duplication](editor-mesh-workspace-copy.md) for the six-sidecar/nine-file export contract and synthetic-input limitations. These bounded point-navigation workflows do not establish arbitrary-game or whole-engine completion.
