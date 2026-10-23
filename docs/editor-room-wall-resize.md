# Resize an existing Room wall in the viewport

Open **Room level authoring**, expand **Manage existing Room walls**, then **Read Room walls** and select a wall. Save pending scene changes and apply or reread any pending wall edits before starting.

1. Click **Resize selected wall in viewport**.
2. Click the first ground-plane corner, then the opposite corner of the desired rectangular footprint. This tool explicitly uses **grid Off**, independently of the new-wall grid control.
3. The second click stages the selected wall’s X/Z center and half-width/depth. Neither click changes the authored scene, history or files. The wall’s GUID, name, Y center and height remain unchanged.
4. Click **Apply Room wall** to commit one existing atomic Room transaction. Admission still rejects overlaps, invalid dimensions and out-of-bounds geometry.
5. Use normal Undo/Redo and Save. Read walls again before another edit.

Escape or **Cancel wall placement** discards the unfinished gesture without changing wall fields. Changing selection, scene, source state or pinned drafts invalidates the gesture. Play, dialogs and other placement tools cannot share its input. Invalid or degenerate second corners do not mutate the selected wall. Existing wall movement, height editing, actor placement and new-wall creation remain separate actions.

Apply checks the current host and exact originally read wall, including name, inside the transaction. A stale read cannot overwrite an intervening change.

## Acceptance coverage

The dedicated `room_wall_resize_ui` test uses the actual production EditorApp with synthetic egui input and software Vulkan. It checks the original narrow footprint leaves the player route open, first/second-corner staging leaves scene/history/files unchanged, Apply creates one transaction, Undo/Redo and Save/fresh reopen preserve wall identity and vertical dimensions, and the expanded footprint blocks that same player route. Screenshots and a structured receipt are retained. This does not establish physical OS-input, hardware-GPU or Windows UI execution coverage.

The additive `room-wall-resize.yml` workflow runs exact-count CPU contracts on Linux and Windows and explicitly executes the otherwise ignored software-Vulkan acceptance on Linux. Existing placement, height, actor and new-wall workflows remain required and unchanged.
