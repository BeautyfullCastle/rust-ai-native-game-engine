# Frame an inspected Terrain patrol route

In TerrainNavigation, inspect the patrol route and choose **Frame inspected patrol route**. The current successful inspection supplies every sampled 3D point: the approach, terrain heights, bends, waypoint legs and loop closure. Showing the route overlay is optional.

Framing changes only the transient view target and distance, adding a margin while preserving yaw, pitch and projection. An all-identical route is centered at the existing distance. Terrain, route settings, simulation Frame, history, selection and saved files remain unchanged.

Inspect again after changing the source or draft. Invalid or stale inspections and conflicting viewport input reject without changing the view. The action requires a current visible viewport and an idle local Edit scene. Unsupported viewport sizes, invalid camera values and fits beyond existing distance or clipping limits also reject.

Focused CPU tests cover complete 3D fitting, admission and preserved state. Windows execution, GPU pixels, physical play and video require separate verification.
