# Inspect a complete Mesh patrol draft

Enable Mesh patrol and enter up to eight waypoints. Use **Inspect patrol draft route** to inspect the approach from the current authored start, every waypoint-to-waypoint leg and the final-to-first connection for a loop. Use the preview display control to show the retained route, or Clear to discard it. Inspection does not Apply or save anything.

All legs must succeed before a report or overlay is exposed. Per-leg distances describe the core's portal-polyline route for a point agent. The aggregate includes the initial approach and, for a loop, one closing connection; it is not a recurring-cycle-only distance or an arrival-time estimate. Projected markers show the actual admitted surface positions, which can differ vertically from authored coordinates within the configured projection range. The path does not establish radius, headroom or step clearance.

The report is bound to the exact patrol form, edit generation, scene/Frame, graph revision, authored start and projection range. Editing, reloading, changing source or leaving the supported Edit context retires it. Observed stale state stays retired even if values are later restored. Validation runs while the panel or preview display is closed. Focus loss retires the report as well. Camera movement alone changes the display, not the retained route.

Review the result, then use the existing Apply patrol and scene Save controls. Undo/Redo, saved-scene loading and runtime patrol behavior remain unchanged. Neither inspection nor drawing writes a gameplay Frame, host history, source file or scene selection.

Focused CPU acceptance compares every displayed leg to independent graph queries, checks malformed/late-leg rejection before any draw command, and exercises real widgets, closed-panel retirement, explicit Apply/save/reopen and runtime regressions. Windows execution, GPU pixels, physical window playback and video remain unverified unless separately recorded.
