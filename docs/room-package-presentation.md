# Manage packages in saved Room presentation projects

The optional editor package manager now admits saved Room character projects through an explicit editor-owned capability. Previously its generic Room opener rejected this declaration, so a project that could author and save presentation could not verify/add/update/remove its packages. Room lighting is part of the scene, so it needs no capability (see [Room lighting](room-lighting.md)).

Build the editor with `package-manager,room-character` for the character route. `room-character` includes the existing Room animated-model route. Without the editor flag, the declaration still rejects even if Cargo has enabled that feature in a dependency. The complete installed lock uses the editor's own `models` and optional `animation` inventory; unified dependency features do not grant consent.

## Saved authoring to package operations

Save the scene (with its lights), model bindings, camera and character documents first. Open **Content packages**, verify installed bytes, then use its existing checked addition, update, removal or restoration workflow. This adds no automatic Save, package migration or new package command. Existing coherent local Edit/dirty/modal/worker guards still apply.

The shared opener validates the editor-local lock inventory and uses explicit Room character capability admission. Character clip maps, asset identity, source generations and numeric/workload/placement limits are admitted through the normal complete Room loader. Referenced package removal remains forbidden; unused package operations preserve the existing saved presentation.

Removal/update/impact/restoration observations now include exact declared character bytes alongside manifest, scene, models and camera. A valid external edit to the character document invalidates an old plan/receipt/report before package publication. Re-preview against the new saved state. Source directory identity, symlink/regular-file checks, package writer and byte verification remain unchanged; this is not hostile concurrent-file isolation.

## Inspect saved usage and run

The package panel's saved-usage worker passes the same explicit editor presentation consent into the shared read-only query. It still verifies the complete lock against editor-local capabilities and reports saved bindings only. The standalone `room_package_usage` CLI retains its previous unsupported presentation defaults; this change does not silently widen that separate consumer.

After package operations, reopen with a matching Room consumer or export with a trusted runtime built for `room-character`. The game reads the same saved character document and the lights of the same scene. Package operations do not write scene/model/camera/presentation bytes or simulation state.

Focused verification covers default-off editor gates under deliberately unified Sample features, complete enabled package lifecycle, valid stale profile rejection, saved-usage consent, legacy package paths, and source-hidden exported native headless gameplay with actual pickup and character poses. This is CPU/headless evidence, not native-window/GPU pixels/audio/video. Existing Windows installation/publication error5 evidence is retained and those operations are not retried; Windows compilation alone is not runtime validation.
