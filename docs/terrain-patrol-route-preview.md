# Inspect a complete Terrain patrol draft

Enable Terrain waypoint patrol and enter or pick up to eight waypoints. Use **Inspect terrain patrol draft route**, then **Show inspected terrain patrol route**. Inspection covers the approach from the admitted authored start, each adjacent waypoint leg and the last-to-first connection for a loop. Clear discards the retained observation. Apply and scene Save remain separate explicit actions.

Every leg must succeed before a report or overlay is available. The actual TerrainGraph projection preserves authored X/Z exactly, samples terrain height and rejects holes, excessive slope or outside-domain points. Overlay markers and lines use those admitted 3D coordinates. Each leg is bounded by the existing runtime's 512-triangle corridor limit; eight waypoints produce at most nine legs.

The displayed graph cost is the existing centroid-based search metric. It is not the polyline's physical length or an arrival-time estimate. The aggregate includes the initial approach and one closing connection. Existing runtime patrol traversal and its wait settings are unchanged.

The observation binds the full form, existing pick generation, scene/host/Frame context, immutable navigation pin, admitted terrain revision and retained terrain-model identity. Row picks, edits, reloads, source changes and focus loss retire it, including while the panel or display is closed. Restoring old values does not revive a retired observation. Reopen of identical bytes is distinguished by the retained model identity. No observation means the per-frame getter skips context admission; retained observations keep the existing full admission checks.

Inspection and drawing do not write a Frame, history, selection or source file. The current runtime-leg overlay remains available independently. Focused CPU acceptance checks exact paths/heights/costs, whole-command atomicity, real widgets, closed-panel retirement, row-pick coexistence and Apply/Undo/Redo/Save/reopen/runtime regressions. GPU pixels, Windows execution, physical window playback and video remain unverified unless separately recorded.
