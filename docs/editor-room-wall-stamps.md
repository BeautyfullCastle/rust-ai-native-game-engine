# Room wall groups

This optional Room-authoring tool captures only ordinary static walls and their Actor, Body and Collider values. It does not copy model/sprite bindings, materials, animation, HUD state, or gameplay actors such as PLAYER, KEY and EXIT. It does not create live linked-prefab instances or introduce a runtime scene schema.

Room wall groups let you save a selected set of ordinary Room walls as a reusable local stamp, then instantiate translated copies in another part of a Room scene. They are expanded actors, not linked prefabs: after instantiation, the scene contains independent wall entities with newly allocated GUIDs and the original wall components. The game schema and runtime do not gain a stamp dependency.

## Workflow

1. Open a local Room project and create or select the walls to reuse.
2. Open **File → Room wall groups…**, read the current Room wall list, and check the walls to capture.
3. Choose a `.room-stamp.json` path in an existing local directory, capture the selection, and explicitly save the stamp.
4. Load a saved stamp when needed, enter an exact fixed-point X/Z offset, and instantiate it. The host admits the complete edit as one transaction/history entry.
5. Save the Room project normally. The instantiated walls are stored in the scene; the stamp file is not needed to reopen or play that scene.

The stamp format preserves wall names and full validated `Actor`, `Body`, and `Collider` component maps. A stamp contains between 1 and 64 walls and is limited to 64 KiB. Destination names must use the `.room-stamp.json` suffix and portable Windows filename characters. Initial saves never overwrite an existing file; replacement requires loading the current file first and is guarded by its observed digest. The store rejects malformed or oversized JSON, non-regular/reparse paths, traversal, and reserved Windows names.

## Safety and limits

Stamp files are trusted local data, not scripts. Capture and instantiation require an idle, editable local Room. Instantiation rechecks the template and current Room admission and submits the expanded actors atomically; a rejected edit does not publish a partial set. Reopening the project does not read or resolve the stamp source.

Filesystem checks use bounded reads and optimistic metadata-stamp/digest rechecks, not persistent Windows file-ID pinning. They detect observed path/content changes but do not provide a capability boundary or a linearizable guarantee against a hostile concurrent filesystem replacement. Keep stamps in trusted, stable local directories. This feature does not implement linked instances, source propagation, per-instance overrides, asset packaging, or a general prefab system.
