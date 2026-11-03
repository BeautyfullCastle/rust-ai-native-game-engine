# Actor sprite sources in a tilemap project

The optional `tilemap` editor can change an existing actor's appearance without
reopening its shared atlas cache. Open an Arena tilemap project, select an actor
that already has a sprite binding, and use **Actor appearance** to choose a
Region or Clip from that binding's admitted sprite document. **Apply actor
source** changes presentation only. It does not change collision, simulation
inputs, Frame state or checksums.

## Closed authoring boundary

Only the binding's source changes. Its GUID, package, document, scale,
orientation and camera-follow settings remain unchanged. The actor set and
combined tile/actor budget remain unchanged. The original atlas textures and
tile compositor are retained; history never reloads or prunes the shared cache.
An existing locomotion binding can be replaced by a Region or Clip and restored
by Undo, but this workflow does not author locomotion pairs.

New actors, atlases, package versions, binding removal and general sprite
Load/Reload/reimport/Close remain unavailable while the tilemap pins its cache.
This is not a general unlock of the Sprite bindings panel.

## History and saving

Use **Undo actor source** and **Redo actor source** for appearance history.
A no-op keeps redo history. Tile history remains separate, and Scene Save does
not save either sidecar. **Save actor appearances** saves the actor sidecar;
**Save tilemap** saves tiles. Save or Undo unsaved appearance changes before
Play or closing, then reopen the complete project to consume saved settings.

The tilemap-owned save checks its original actor bytes and existing project,
lock, tilemap and installed-asset admission before publication. Only a successful
explicit save updates the actor saved marker and the tilemap's pinned actor
bytes. External edits are not silently adopted. Saving actor appearances does
not rewrite the tilemap, packages or manifest; subsequent tile edits retain
their existing save checks.

Edit-mode, source/scene-epoch and read-only guards remain in force. Invalid
regions/clips, unavailable actors and stale sources are errors, not fallback
appearances. Prepublication failure preserves the previous file and saved
state. Publication is a single-file operation under trusted, quiescent local
directories, not an atomic multi-file project transaction or hostile-filesystem
sandbox. A reported postpublication uncertainty requires inspection, not blind
retry or a rollback assumption.

## Acceptance boundary

Focused checks exercise same-asset validation, history/no-op behavior, source
mutation rejection and coordinated saving. Actual headless Editor acceptance
checks widget Apply/Undo/Redo, unchanged tile/atlas pixels and authoritative
state, Save/reopen and Play with the selected sprite source. Existing package
and export admission remain required. Software offscreen results do not prove
native-window input, physical GPU or Windows compatibility.
