# Reusable mesh prop assets in the editor

MeshNavigation can save an applied static prop as a local reusable asset and load it into another admitted mesh graph. This is an editor-only workflow under `mesh-navigation`; it uses the existing prop history, sidecar, and export format. It does not introduce a runtime prefab system or linked instances.

## Use

1. Open a local MeshNavigation scene in Edit mode. Import a licensed static prop package, apply its placement/material settings with **Add prop** or **Update selected prop**, and select that applied prop.
2. In **Mesh props → Prop prefab**, enter an absolute local **Prop prefab file** path ending in `.mesh-prop-prefab.json`. Its parent directory must already exist.
3. Choose **Save selected prop prefab**. The worker captures only the applied selected prop, not an unapplied form or a viewport preview. Saving never overwrites an existing asset and does not create a pending candidate or change the scene.
4. Open another admitted local mesh scene. Enter the saved asset path, explicitly check **I have reviewed rights to use this prop prefab**, then choose **Load prop prefab**.
5. Loading validates the embedded source, license, appearance, and original graph identity before rebinding the cosmetic snapshot to the current admitted graph. It stages a candidate only: the prop document, simulation Frame, and history remain unchanged.
6. Choose **Add prop** to assign a fresh stable prop ID, or **Update selected prop** to retain the selected ID. A colliding name receives an available bounded `Prefab N` name. Alternatively choose **Discard loaded prop prefab** to restore the applied selection fields.
7. Use **Undo prop edit** and **Redo prop edit**, then **Save props** or **File → Save mesh workspace**. Reopen and Play the saved scene normally. The saved prop sidecar embeds the admitted model and license, so the prefab file and original package are not required at runtime or in an exported game.

## Admission and safety

- One worker performs bounded local file IO and codec work at a time. Scene, scene-load epoch, graph revision, path, input generation, and rights acknowledgement must still match when a loaded candidate is returned.
- Worker completion is polled even with the section collapsed. A stale load never attaches a candidate. An explicitly saved asset can remain on disk after a context change; its completion does not mutate the new scene.
- Worker activity blocks conflicting prop/viewport mutations, history, workspace switching, saving, and Play. A loaded candidate blocks saving and Play until explicitly added, updated, or discarded. Editing a file path or rights checkbox alone is not a scene edit.
- The strict version-1 wrapper is at most 16 MiB. Its `art_json` string preserves nested duplicate-field evidence for the existing strict Snapshot decoder. Names must be trimmed, nonempty, at most 64 UTF-8 bytes, and contain no controls.
- The asset preserves source model bytes, source/license hashes and text, transforms, tint, UV/pivot, facing, and per-material-slot overrides. Only the in-memory target graph revision is rebound after original identity validation.
- Paths must use existing ordinary local directories. UNC/device paths, alternate data streams, nonportable/reserved names, symlinks, and Windows reparse points are refused. Save uses a synchronized staging file and atomic no-clobber publication.
- This is a trusted stable local-root workflow, not isolation from a hostile concurrent filesystem writer or a signature/authenticity service. Rights acknowledgement is explicit user review, not a legal determination.

## Scope and verification

Existing limits still apply: at most six cosmetic static props per mesh scene, bounded prop sidecar/derived model data, and point-agent navigation. Prefabs do not add collision, skinned characters, linked updates, dependency watching, or runtime instantiation. Existing linked-prefab tooling for CollectDodge is a different game-specific feature.

Acceptance covers strict codec/store/worker cases and the real editor path: save an applied prop asset, load into a different graph, explicit Add, Undo/Redo, save, reopen with prefab/package hidden, Play/mission replay, and source-free export/native execution. Implementation and validation evidence are recorded separately in PR205; this document alone is not a claim that every acceptance step or the whole engine is complete.

The preserved PR205 history through `2122d52a1056988479fe46de5a94b01f6c30eb00`, including original failures and limits, is in [the Windows authoring history](verification/pr205-windows-authoring-history.md).
