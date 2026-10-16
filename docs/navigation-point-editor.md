# Author and play one terrain point agent

This optional production-editor slice uses the existing terrain-triangle graph,
canonical shared-edge portals, deterministic Dijkstra route, and fixed-point
portal-midpoint movement. It does not implement clearance-aware navmesh baking,
funnel smoothing, crowds, walls, imported obstacles, or a physics character.

## Run

```sh
cargo run --release -p orr_editor --features navigation -- \
  --game navigation-yard3d --scene scenes/navigation_blank.scene.yaml
```

Use a copy of the scene in a working project directory when making content. A
navigation host stays confined to its original scene directory; start another
host to open a scene in a different directory. `navigation_point.scene.yaml` and
`navigation/point_demo.orrt` provide a built, source-pinned example.

## Authoring loop

1. In Terrain authoring, create or open a terrain with at most 17 × 17 vertices.
   Save it to a scene-relative `.orrt` file. Installed terrain packages remain
   immutable: use the explicit copy-to-scene action before building a route.
2. In Point navigation, choose fixed-point start and goal X/Z, maximum rise/run
   slope, and a positive distance per tick. Radius, headroom, and max step are
   exactly zero. At most one agent and 512 terrain triangles are admitted.
3. Choose Build route. The editor submits one complete reflected scene pin, with
   both full SHA-256 revisions and the agent specification. Host admission
   independently validates the source, graph, complete route and limits. A
   rejected Build leaves the old authored scene and Frame unchanged.
4. Save the scene. Closing and reopening restores the exact pin and agent setup.
5. Use the normal Play, Pause, Step, Seek and Stop controls. Position and movement
   come from the host Frame, not the egui clock. Stop restores the authored
   scene's starting state.
6. Stop before terrain or profile changes. Save changed terrain and explicitly
   Build route again. The old route is stale until rebuilt; editor Play is
   blocked. Terrain Undo/Redo uses the existing terrain document history;
   complete route Builds use host scene history. Exact Undo restoration may reuse
   a route only when terrain bytes, source binding, scene and every agent/profile
   field match the admitted identity; a changed identity still requires Build.
   Saving a changed pinned terrain keeps the stale warning even if its authoring
   panel is closed. Discarding an unsaved edit may return to the admitted route.

This first editor navigation slice requires every terrain vertex, route point,
and agent anchor to be inside the inclusive −256…256 range on X, Y and Z. Every
Q48.16 lattice point in that range converts exactly to f32, so intermediate
movement stays representable as well as the authored endpoints. Terrain heights
and projected route segments remain inside their vertex bounds. Build validates
the complete candidate presentation before its single host edit and rejects
out-of-range input atomically, without clamping.

The renderer-free host and navigation core retain their wider numeric contract.
Opening a manually authored wider scene reports an explicit editor view error
and blocks editor Play. Ordinary non-navigation terrain presentation retains its
existing exact-f32/±1,000,000 contract.

The shared viewport draws one terrain model and one terrain-free overlay model
for accepted triangles, corridor, route and agent, all in the same depth pass.
The bounded overlay mesh is rebuilt when the agent position changes; this slice
does not claim optimized per-tick GPU upload cost or large-agent scalability.
Edit mode shows the local working terrain; Play shows the
admitted Frame terrain. Terrain and navigation clocks are not separate.

## Ownership and failure behavior

- Admission may read one confined source path. It rejects traversal, symlinks,
  missing/oversized/malformed assets, wrong identity or revisions, unreachable
  routes, and every authored physics body/collider/entity. A pristine blank
  scene is an Edit-only staging state. Like the existing terrain admission,
  component-by-component path checks reject static symlinks but do not provide
  race-free protection against a concurrent filesystem path swap; that stronger
  OS-specific file-descriptor walk is outside this slice.
- Terrain bytes, graph bytes, navigator snapshot, agent specification and all
  persistent handles live in registered Frame POD/list state. A copied Frame
  contains all authority required for replay and seek.
- After successful admission, changing or removing the source file does not
  change an already admitted Play session. A later scene edit/reopen must pass
  admission again. Views also reconstruct from the admitted Frame.
- A failed runtime step changes only the authoritative failure status and halts
  subsequent movement. There is no fallback floor, route, or hidden actor.
- Snapshot restore validates dependency identity, canonical route, status,
  cursor, terrain position and safe continuation. It does not authenticate a
  historical movement transcript supplied by an untrusted caller.

## Package boundaries

`orr_navigation` remains GPU-free query/movement code.
`orr_navigation_runtime` owns the bounded Frame adapter. `orr_games/navigation`
and `orr_remote/navigation` add the game and host. `orr_editor/navigation` opts
into terrain presentation and the navigation host. Default and terrain-only
builds have no navigation edge. Navigation does not enable terrain physics,
animated models or irradiance probes, and does not add dependencies to FP, ECS,
sim, session, or the physics solver.

## Validation gates

Focused core/runtime and adversarial restore tests, source admission and
filesystem-independent host replay, actual EditorApp authoring/lifecycle tests,
mandatory software-GPU readback/captures, dependency guards, and affected strict
Clippy are separate from exact-source independent review and required remote CI.
Software-GPU/headless evidence does not establish native-window, physical-GPU,
Windows, macOS, or full-workspace validation. See the implementation PR for the
exact tested source, commands, original failures, captures and remaining limits.
