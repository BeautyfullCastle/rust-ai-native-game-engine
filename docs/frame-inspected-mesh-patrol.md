# Frame an inspected Mesh patrol route

In MeshNavigation or MeshHazards, inspect the waypoint patrol and choose **Frame inspected patrol route**. The action uses the current successful inspection, including its projected start, every waypoint, every path bend and the closing loop leg. The route overlay may be shown or hidden.

Framing adjusts only the transient view target and distance with a margin. Yaw, pitch and the authored projection policy stay unchanged. Orthographic framing uses the existing session zoom behavior. An all-identical stationary route is centered without changing distance. Simulation Frame, route settings, history, dirty state, authored camera and files remain unchanged.

Inspect again after changing the route or source. Failed, retired or stale inspections cannot be framed, and old queued requests cannot revive after values change back. Framing requires the current visible viewport and an idle local Edit scene. Follow, static-camera preview, picking, drags, capture and conflicting work must be finished first. Unsupported sizes, invalid camera values and fits beyond existing distance or clipping limits are rejected without changing the view.

Focused CPU tests check all route points and screen margins across projections and viewport aspects, real inspection/button/request handling, stale rejection and preserved state. Existing framing and relevant input/inspection regressions are retained. GPU pixels, Windows execution, physical play and video remain unverified unless separately demonstrated.