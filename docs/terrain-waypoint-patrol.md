# Saved TerrainNavigation patrol

Build the editor with `--features terrain-patrol`. Open a local NavigationYard3D
scene, admit its ordinary terrain route, then expand **Terrain waypoint patrol**.
Enable patrol, set up to eight X/Z waypoints, choose looping and complete wait
ticks (0..3600), and click **Apply terrain waypoint patrol**. Apply is one scene
edit; ordinary Undo/Redo and Save persist the authored route. Draft fields do not
change the scene or agent. Reload explicitly after a scene, source, graph or epoch
change. Pending terrain/route edits, unfocused controls and read-only hosts cannot
apply patrol.

Admission checks the route from the authored start, every subsequent leg and the
closing leg when looping, including slope, coordinate bounds and pinned graph
revision. Terrain projection must retain the exact authored X/Z; out-of-surface
points reject rather than snap to a nearby route. Each waypoint waits the requested
number of whole simulation ticks before the next leg begins. Nonlooping patrol
completes after its final wait. There is no manual-goal override in this game.

The authored scene pin remains immutable. Runtime cursor, wait, arrival, completion
and loop state belong to the rollback Frame and cannot be authored. Replanning
reuses the existing agent and three runtime list handles. A failed attempted tick
discards its candidate movement/progress before recording failure. Reset/seek/replay
use Frame state; they do not reread terrain files.

Saving and reopening a scene still requires its original confined `.orrt` source
for a fresh bake. Once admitted, Play, views and the authored reader's restart can
continue from embedded Frame bytes after that source is hidden. The opt-in game
and host profiles must match. Feature-off registries and build identity remain
unchanged; patrol uses a distinct build identity.

The legacy point and surface bundle formats do not support this compiled patrol
profile, including scenes where patrol is disabled. Their capture APIs reject it
before reading the source. Use editor Play or a matching authored reader; this
change does not add a patrol bundle format.

Acceptance covers exact waits, loops/completion, slope/unreachable routes, atomic
rejection, source-bound UI Apply/Undo/Redo/Save/reopen, same-path epoch retirement,
source-hidden replay/restart and stable entity/list handles across many loops.
The connected desktop is locked. CPU tests do not verify GPU appearance, physical
window input or playtest video.
