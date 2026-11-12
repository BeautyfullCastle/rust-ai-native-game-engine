# Focus the editor viewport

Click **Focus viewport** above the editor toolbar to hide the hierarchy,
inspector and bottom authoring panel. **Restore panels** remains in the same
place. Playback buttons, connection warnings, status and project-owned game
controls remain available. The ordinary panel layout is the default.

This is a window-only preference, shared by all editor games. Returning keeps
panel sizes, the chosen bottom tab, hierarchy filter and selection. The toggle
does not start, stop, reset or reconnect a host and does not edit the scene or
undo history. Stopping playback does not reset the layout choice.

Existing input safety still applies: clicking outside the game viewport may
release gameplay control. Escape retains its existing gesture/input cancellation
meaning; it is not a new layout shortcut. Standalone player windows are unchanged.

Acceptance covers repeated enter/exit, edit and playback modes, retained panel
state, and actual 960×720 offscreen WGPU captures of Stormglass and another
registered project. Board pixel area must increase, not just the surrounding
letterbox. Offscreen software rendering does not certify native hardware/windows.
