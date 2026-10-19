# Room wall groups

This optional Room-authoring tool captures only ordinary static walls and their Actor, Body and Collider values. It does not copy model/sprite bindings, materials, animation, HUD state, or gameplay actors such as PLAYER, KEY and EXIT. It does not create live linked-prefab instances or introduce a runtime scene schema.

Room wall groups let you save a selected set of ordinary Room walls as a reusable local stamp, then instantiate translated copies in another part of a Room scene. They are expanded actors, not linked prefabs: after instantiation, the scene contains independent wall entities with newly allocated GUIDs and the original wall components. The game schema and runtime do not gain a stamp dependency.

The optional `room-authoring` feature enables this panel on Windows and Linux x86_64. Other targets remain gated. Translation is supported; quarter-turn rotation is not part of this feature.

## Workflow

1. Open a local Room project and create or select the walls to reuse.
2. Open **File → Room wall groups…**, read the current Room wall list, and check the walls to capture.
3. Choose a `.room-stamp.json` path in an existing local directory, capture the selection, and explicitly save the stamp.
4. Load a saved stamp when needed, enter an exact fixed-point X/Z offset, and instantiate it. The host admits the complete edit as one transaction/history entry.
5. Save the Room project normally. The instantiated walls are stored in the scene; the stamp file is not needed to reopen or play that scene.

The stamp format preserves wall names and full validated `Actor`, `Body`, and `Collider` component maps. A stamp contains between 1 and 64 walls and is limited to 64 KiB. Destination names must use the `.room-stamp.json` suffix and portable Windows filename characters. Initial saves never overwrite an existing file; replacement requires loading the current file first and is guarded by its observed digest. The store rejects malformed or oversized JSON, non-regular/reparse paths, traversal, and reserved Windows names.

## Safety and limits

Stamp files are trusted local data, not scripts. Capture and instantiation require an idle, editable local Room. Instantiation rechecks the template and current Room admission and submits the expanded actors atomically; a rejected edit does not publish a partial set. Reopening the project does not read or resolve the stamp source.

Filesystem checks use bounded reads and optimistic metadata-stamp/digest rechecks. Windows retains its existing reparse-point checks and does not pin persistent file IDs. Linux x86_64 opens files and directories without following final-component symlinks, uses nonblocking opens to reject substituted FIFOs without waiting for a writer, and compares device/inode identity and file metadata around reads. Existing symlink ancestors and dangling symlinks are rejected. Hardlinked regular files are allowed; replacing a stamp replaces that directory entry, leaving other hardlinks on the previous contents.

These checks detect observed changes but do not provide a capability boundary or a linearizable guarantee against hostile concurrent replacement. Directory ancestry is checked by path, not held as a pinned directory capability. Replacement has a gap between the final digest check and publication: a writer in that gap can be overwritten. Initial publication uses no-clobber semantics and fails if the destination appears. Keep stamps in trusted, stable local directories. This feature does not implement linked instances, source propagation, per-instance overrides, asset packaging, or a general prefab system.


## Acceptance coverage

The additive Room wall groups CPU and Linux UI acceptance workflow requires the original template/editor tests and Windows store tests, Linux filesystem regression tests, and an explicitly executed Linux production EditorApp test using software Vulkan. The unchanged Windows UI test additionally requires real hardware Vulkan and explicit `ORR_REQUIRE_GPU=1` or `ORR_ROOM_WALL_STAMPS_ACCEPTANCE=1`; ordinary hosted Windows CPU runs do not establish that proof. A successful Windows hardware UI run remains required before merge; this workflow does not invent an unavailable GPU runner or weaken that requirement. Exact test counts reject empty or skipped runs. The Linux tests include regular, symbolic and hard links, dangling links, invalid ancestors, nonregular files including FIFOs and Unix sockets, byte limits, malformed data/digests, stale replacement, and deterministic read/publication races. Receipts and screenshots are retained by the workflow.
