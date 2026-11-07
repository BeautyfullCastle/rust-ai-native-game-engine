# Pick Terrain patrol waypoints in the viewport

Enable Terrain waypoint patrol and add the rows you need. Choose the pick action beside a waypoint, then click a walkable terrain cell. The selected cell center replaces that row's X/Z draft. Other rows, loop and wait settings remain unchanged. Cancel or Escape ends picking.

Picking does not Apply or save the scene. Review the complete draft, use the existing Apply terrain patrol action, then save the scene explicitly. Apply validates every leg, including the closing leg for loops. Existing Undo/Redo, saved-scene loading and TerrainPatrol native delivery remain the execution path.

Picking requires a focused, editable local TerrainNavigation scene with current saved terrain and an admitted navigation graph. Holes, blocked cells, unsupported coordinates and stale input reject. A change to the source, graph, scene or patrol draft retires the pending pick; removing a row cannot redirect a later click into another waypoint. Play mode, competing pointer tools and focus loss cancel picking. An owned cancelled click stays consumed through release.

CPU acceptance covers fixed-point cell centers, walkability, draft pin retirement, real widget requests, viewport input dispatch and Apply/save/reopen/runtime behavior. These checks do not establish Windows filesystem execution, GPU rendering, a physical game window or playback video.
