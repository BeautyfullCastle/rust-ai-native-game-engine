# Room controls in editor Play

Build the main editor with `--features room-input-map-play`. This separate,
default-off capability includes `room-project` and the existing standalone
`room-input-map-editor`. The standalone tool remains available without the main
editor integration. Builds without the new capability keep the existing Room
WASD/arrows and E input route.

## Author and play

Opening a complete Room project through the CLI or supported project-open UI
installs its admitted controls bytes. A project without controls uses the Room
defaults. The editor pins that snapshot in memory and does not reread it during
Play or Restart/Reconnect.

In idle local Edit mode with control released, expand **Room controls for Play**.
Use the existing five-action editor to open or author a map. **Apply action**
accepts the selected action draft; **Use for Play** explicitly installs the whole
accepted map for this editor session. Pending action drafts block Use for Play.
Undo, Redo, Defaults and Open change the authoring document, not the active Play
map, until Use for Play is clicked. The visible hint always describes the active
admitted map.

Press Play and Take control. Physical keyboard identity is required. Missing identity releases control and
quarantines held keys instead of guessing from logical keys. All authored
aliases work, including Tab/Space. Escape releases editor control. Unbound Tab
also releases. R remains reserved and the existing Restart room button is used
for restarting. Ctrl/Command/Alt key presses do not start gameplay actions.

Focus loss, dialogs, proposal preview, scene changes and released control disarm
held input. A held key must be released and freshly pressed before it can move
or interact after reclaim. Physical Escape is resolved before pumping the host;
a mapped physical key reported as logical Escape does not release control.

## Persistence and checks

An explicit session override survives ordinary Play/Stop. Reload or host restart
of the same scene restores the admitted project snapshot. Switching to another
scene drops that snapshot and uses defaults; opening a complete project installs
its newly admitted snapshot. Authoring drafts are retained across scene reloads.
The existing complete-project replacement route replaces the whole editor.

No project manifest or controls file is changed automatically. Save As remains
an explicit, no-overwrite controls-file operation. Reopen the complete project
to admit an externally changed project controls file.

Focused CPU tests cover physical key mapping, opposite directions, draft
rejection, quarantine, actual Room-host movement and interaction, restart,
production widgets, and full-editor logical/physical Escape handling. These are
not native-window, physical-device, GPU, Windows execution, or full workspace
acceptance evidence.
