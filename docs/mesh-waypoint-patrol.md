# Saved MeshNavigation waypoint patrol

Build the editor with `--features mesh-patrol`. Open a local MeshNavigation scene with an admitted agent, then expand **Waypoint patrol**. Enable the route, edit each waypoint's fixed-point X/Y/Z coordinates, add or remove up to eight points, choose looping and wait ticks, then click **Apply waypoint patrol**. Save the scene with the ordinary scene Save command. Undo/Redo use the existing scene edit history.

Staging fields does not change the scene. Apply validates the entire route against the current graph before one atomic edit: initial position to the first point, every subsequent leg and the closing leg for a loop. Coordinates are bounded to -256..256; wait ticks are integers from 0 to 3600. Stale scene, graph or source drafts must be explicitly reloaded before applying. Disable the patrol before replacing or reprojecting its graph: routes are never silently rebound to another graph revision.

Play automatically targets waypoint 1. On arrival it waits the specified number of complete simulation ticks, then targets the next waypoint on the following tick. A nonlooping route completes after the final wait; a loop returns to waypoint 1. All cursor, wait and pause state belongs to the deterministic Frame and restores with rollback. Invalid state fails closed without retaining a partially attempted movement tick.

A successful manual goal request pauses automatic patrol, including a request for the current goal. The requested manual route still moves normally. Invalid or unreachable requests do not pause the patrol. Restarting the saved scene restores its authored patrol from the beginning; there is no automatic resume command in Play.

Only the authored route is saved; runtime progress cannot be authored. The ordinary single-goal configuration remains available when patrol is disabled. `mesh-patrol` has distinct host build profiles and requires matching game/host feature support. Feature-off registries and legacy checksum values remain unchanged.

This feature controls the existing single point agent. It does not establish radius, headroom, clearance or multi-character navigation. The separate MeshHazards game explicitly rejects enabled patrol because it uses a different movement system. Existing point-agent mission support can be compiled alongside patrol.

Acceptance covers exact wait/arrival ticks, loop/completion/manual goals, malformed and unreachable routes, atomic rejection, snapshot replay, and actual editor Apply/history/save/reopen/Play. The connected computer is locked; native window input, GPU appearance and playtest video are unverified. CPU/UI harness tests are reported separately.
