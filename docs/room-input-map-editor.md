# Room input-map editor

Run `cargo run -p orr_editor --features room-input-map-editor --bin orr_room_input_map`.
This default-off standalone window authors five Room actions. Main-editor Play
uses the separate opt-in [room-input-map-play](room-controls-play.md) capability.
It does not change a project manifest or launch the game.

Select an action and choose physical keys using its binding dropdowns. Add/Remove
edit a separate draft. R and Escape remain reserved for restart and exit; every
action needs 1–8 keys and keys cannot repeat within or across actions. Apply
validates the complete candidate with the production `RoomControls` adapter.
Invalid drafts stay visible without changing accepted state or history.

Apply or Discard the draft before changing action, Open, Undo, Redo or Defaults.
Defaults and successfully opened maps are undoable. History holds 32 accepted map
edits. Repeated Apply/Open of an unchanged map does not create history entries.
Save As always writes only the accepted map and preserves a pending draft.

Choose a new simple ASCII `.json` filename, starting with a letter or digit and
using only letters, digits, `_`, `-`, and `.` (maximum 128 characters). Manifest
and lock filenames are reserved. Existing files are never overwritten. Open uses
the shared bounded local loader, rejecting symlinks and special files; Save As
uses `ActionMap::save_new` and its no-replace hard-link publication. Filesystems
without hard-link support fail rather than overwrite. A cleanup failure may be
reported after publication; inspect the destination before retrying.

Save into the Room project root, with a filename distinct (ignoring case) from
all other entry documents. Explicitly set `"controls": "room.controls.json"` in
the `entry` object of `orr.project.json`, using your chosen filename. Build the
Room player/exporter with `orr_sample/room-input-actions` plus their normal Room
features. This editor does not attach the file automatically. Reopen the saved
file through Open to inspect it; loaded changes can be undone.

Acceptance: `cargo test -p orr_editor --no-default-features --features
room-input-map-editor --bin orr_room_input_map --test room_input_map_authoring_ui`.
CPU tests cover validation, bounded history, safe persistence and saved-map Room
input interpretation. Headless egui tests drive the production widgets for
Apply, Discard, undo/redo, defaults, Save As/reopen, duplicate rejection, pending
navigation and repeated clicks. They do not prove native OS keyboard handling,
window rendering or GPU behavior. Main-editor Play has separate acceptance checks
documented in [Room controls in editor Play](room-controls-play.md).
