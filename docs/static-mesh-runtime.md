# Frame-owned static-mesh agent workflow (partial #103)

`orr_navigation_runtime::mesh::MeshRuntime` connects the optional
[`StaticMeshGraph`](static-mesh-navigation.md) to real ECS state rather than an
external heap-only agent. It supports one explicit point-agent route and stores
its geometry, profile, dependency pins, request, distance cursor, position and
movement status inside `orr_ecs::Frame`. Clone, rollback, checksum and serialized
Frame restoration therefore include everything authoritative to this adapter.

This is not a new default engine dependency or game schema. Consumers opt in by
registering these types in their own registry and calling the adapter from their
command/tick path. Existing terrain navigation, editor and remote game types are
unchanged. No renderer, physics solver, filesystem or clock is consulted by ticks.

## Create, move, stop, save, reload and replan

Run the complete Windows-local workflow from a checkout:

```powershell
cargo run --release -p orr_navigation_runtime --example frame_mesh -- --output "$PWD\bridge.frame"
```

The example bakes a ramp and raised bridge, admits a bound route to a Frame, moves
and stops the agent, saves a create-new Frame snapshot, reloads it with the same
registry and expected pins, explicitly replans, and reaches its goal. It compares
per-tick checksums and complete serialized Frame bytes during replay and rollback
continuation. `--help` documents the invocation. Existing destinations are never
replaced; the output is not a transactional project export and may remain partial
on write/sync failure. This is a CPU/Frame workflow, not an editor/GPU/physics test.

The example's world is defined only by its explicit walkable triangle fixture, so
its world revision equals that fixture's graph revision. This does **not** prove a
separate collision world matches the navigation geometry.

## Consumer API and dependency boundary

Register `MeshRuntime::register(&mut builder)` once before constructing the Frame.
Its `AssetByte` list pool is a distinct element type, not the terrain adapter's
shared `u8` pool. Both navigation adapters can coexist in the same registry.
Operations on a registry missing required types fail with `Unregistered` instead
of calling typed Frame accessors that would panic.

`WorldBinding` holds two explicit pins:

- `graph_revision`: the expected canonical static graph revision.
- `world_revision`: a nonzero, caller-owned world/collision dependency revision.

`admit(frame, graph, binding, spec)` checks these pins, all runtime bounds, the
canonical graph and the complete route before allocating Frame-owned data.
`AgentSpec` supplies 3D start/goal, maximum vertical projection tolerance and a
nonnegative `distance_per_tick` route parameter. The runtime derives the initial
position and status rather than trusting externally supplied derived fields.

Pass the **currently expected** binding to every `validate`, `agent`, `step`,
`stop` and `remove` call. A different binding fails even for stopped/arrived agents
or a zero movement increment. The stored graph must also reproduce the expected
graph revision. Silently reading the expected binding from an untrusted snapshot
would defeat dependency checking; the consumer must supply its independently
known current pins. Pins detect mismatch, not authenticity or trusted history.

The runtime cannot inspect an application's collider/editor world to calculate
or certify `world_revision`. Updating that revision when relevant geometry changes,
choosing the correct walkable triangles and establishing clearance remain consumer
responsibilities. Matching caller-supplied labels alone do not prove physics
agreement, obstacle safety or game-world freshness.

## State, movement and explicit replanning

`RuntimeState` owns the graph byte-list handle, agent entity and dependency pins.
`RuntimeAgent` owns the request, current position, absolute distance cursor, status
and a zero reserved field. These are fixed-layout Pod types; no heap pointer is
stored in a component. Routes and temporary graph helpers are reconstructed from
bounded Frame bytes on each call, with no external authoritative cache.

Validation checks entity generation/count, list lifetime, graph encoding/bounds,
binding, request, cursor range, canonical sampled position and status. Unknown
status or nonzero reserved fields fail. `agent` returns a read-only derived path,
position and status for a consumer's view; the view does not become authoritative.

`step` advances the **absolute route parameter** by `distance_per_tick`, clamps to
the path length and publishes the derived position/status only after validation.
It does not change `Frame::tick`; the game owns tick advancement. This is not the
terrain adapter's physical-distance-budget contract: lattice projection can make
endpoint displacement differ by raw units. No incremental speed cap, swept
collision or character-controller behavior is claimed.

`stop` retains the admitted route/cursor and freezes movement. There is no implicit
resume or reroute. `replan(frame, graph, new_binding, goal, max_vertical,
distance_per_tick)` first validates the old Frame state, then explicitly admits
the new graph/binding and a route from the current position. It preserves the
entity, resets the cursor and derives the new projected start/status. Vertical
projection within the explicit tolerance may change Y; XZ is never snapped.
An unreachable goal, stale/malformed old state or invalid new dependency leaves
the Frame unchanged. Changing graph/world pins is allowed only through this
explicit replan, not silently during a tick.

`remove` validates the current binding and state, frees the owned byte list,
despawns the agent and resets the singleton. A deleted/reused entity or stale list
handle cannot be mistaken for the prior agent. Returned errors from admission,
stepping, stop/replan and removal do not publish partial simulation state.

## Bounds, persistence and remaining acceptance

The adapter supports at most 1,024 canonical vertices, 512 **source** faces
(including rejected slopes), and 30,768 graph bytes. Header counts and available
bytes are checked before graph reconstruction. Search has a 512-expansion bound.
The larger standalone mesh limits remain unchanged. All point/open-sky slope and
geometry restrictions of `StaticMeshGraph` still apply.

Use existing `Frame::to_bytes`, `Frame::from_bytes`, clone/copy and registry schema
checks rather than a second snapshot format. Derived positions being consistent
with a cursor are not proof that trusted commands produced that cursor. Authenticate
snapshots or replay trusted input history where provenance matters.

This slice does not modify the reserved navigation editor/remote-game/exporter
paths. Actual editor scene save/reopen/play, independent collision-world binding
and source-independent Windows export remain separate acceptance gaps. Neither
this optional adapter nor a passing replay establishes full #103/#94 completion.
