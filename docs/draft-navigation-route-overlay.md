# Show an inspected draft navigation route

Mesh navigation authoring can display a route before placing or moving an agent. It is an optional read-only overlay of the same bounded point-agent inspection used by **Inspect draft route**.

1. Create or open the intended mesh graph and finish draft geometry fields or gestures.
2. Set Start, Goal, maximum vertical projection and slope inputs.
3. Click **Inspect draft route**. Review both endpoint projections and the result.
4. For a successful route, enable **Show inspected draft route**.

The overlay submits a cyan polyline plus green start and red goal markers to the existing viewport line renderer. It follows the validated portal polyline, including detours and height. The default is off, and a new inspection resets the display option. Failed, invalid or disconnected inspections have no route display. A coincident start and goal can have overlapping markers.

The overlay does not place or authorize placement, move an agent, send host commands, edit the Frame/history/selection, or write a scene or asset. Point-agent reachability is not radius, headroom or clearance validation, and the polyline is not a shortest-path guarantee. Existing agent placement remains a separate explicit action.

Every paint checks the loaded scene and epoch, graph revision, exact eight draft field strings, selected asset/model sources, mode and geometry/gesture state. A stale result is suppressed even while the panel is closed. Once a paint observes it as stale, restoring the old inputs does not revive it; inspect again. Play, previews or unfinished geometry/viewport gestures also suppress the inspection. Display uses the currently admitted graph and does not follow external file edits automatically.

Drawing converts and validates all points before adding any commands. Invalid, empty, oversized or nonrepresentable input produces no partial overlay. The data remains fixed-point in the retained core path; float coordinates are presentation only. Existing viewport depth/occlusion behavior applies, so the overlay is not an x-ray or collision diagnostic.

Focused tests compare exact route segments, endpoint marker commands, colors and point bounds, including one-point and maximum-size paths. Actual UI acceptance inspects/toggles a bent elevated route, invokes the production paint adapter and compares it with an independent core path calculation. It checks unchanged scene state/history/files and failed/stale suppression. Existing route diagnostics remain regression-tested.

The App render hook is source-reviewed and compile-checked, including the navigation/terrain feature combination. CPU command-list checks do not establish actual GPU pixels or native window/input/audio/video. The connected computer remains locked, and historical Windows publication failures are not retried. No playtest video is claimed without a separate real playtest.
