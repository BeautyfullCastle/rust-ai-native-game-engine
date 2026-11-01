# Inspect a mesh draft route

In the optional MeshNavigation editor panel, create or open a mesh, edit Start,
Goal and Maximum vertical projection, then choose **Inspect draft route**.
The report distinguishes Start and Goal projection failures, disconnected
triangles and exhausted search budgets. A successful report shows projected
positions, triangle IDs, corridor size and the validated fixed-point path length.

This inspects the loaded draft, not a file that has been changed externally since
opening. Reopen to refresh external changes. Unopened source selections, unstaged
geometry fields, active gestures and stale staged geometry block inspection.
Changing the draft coordinates, graph, scene or scene-load epoch discards the
displayed result; inspection is only recomputed when requested.

The diagnostic cannot place or move agents, write files or change editor history.
It is not placement authorization. It uses the existing bounded point-agent
triangle graph and portal-midpoint path, not an optimal funnel path. Radius,
headroom, step clearance and dynamic obstacle avoidance are not certified.

Focused acceptance covers each invalid endpoint, disconnected islands, valid
routes and lengths, search budgets, malformed inputs, stale report disposal,
actual egui controls and unchanged live-editor history/checksum/selection/files.
No navigation core, scene schema or simulation contract changes are required.
