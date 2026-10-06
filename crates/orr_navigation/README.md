# Bounded terrain navigation (partial #103)

`orr_navigation` is optional, GPU-free fixed-point code. It does not change any
engine default or add a simulation/game dependency. This slice is a canonical
heightfield triangle graph with deterministic corridor planning and an explicitly
snapshotted movement helper. It is not a general navmesh.

## Supported agents and numerical domain

- Y-up, continuous XZ heightfield, point agent in open sky
- `AgentProfile::max_slope` is a rise/run ratio, not degrees
- Radius, headroom and maximum step must be exactly zero. Any other value is
  rejected; clearance, ceilings, discrete steps and collision are unsupported
- Spacing raw FP values must be 16 through 2^32 inclusive
- Every terrain vertex's X, Y and Z must be within ±2^40 raw inclusive
- Maximum slope raw must be 0 through 2^24 inclusive
- Terrain retains its existing 2–129 vertices per side limit

These bounds admit sub-millimetre grids and ±16,777,216 world units while keeping
all exact coefficient products safely inside i128. Tiny spacing and extreme
coordinates/heights fail explicitly. There are no floats, unordered containers,
wall clocks, randomness or architecture-sized serialized state.

## Graph, projection and planning

Terrain vertex IDs are unchanged, including vertices adjacent to holes. Triangle
keys are `(row-major cell index * 2 + half)`: half 0 is `[a,c,d]`, half 1 is
`[a,d,b]`. Holes and rejected slopes leave stable gaps. Each triangle's actual
plane rise coefficients are used to compare
`(hx² + hz²) * 65536² <= spacing² * max_slope_raw²` exactly. Terrain's quantized
normal/slope diagnostics are not admission criteria.

Only actual full shared edges between two admitted triangles produce portals.
Their sorted vertex IDs and neighbor keys are canonical and reciprocal.
Corner-only contact creates no edge. Portal XZ midpoints floor to the raw lattice;
Y is sampled from terrain. Midpoint canonical ownership is checked independently.

Projection uses the exact supplied XZ: internal seams belong to the +X/+Z cell,
the closed outer edges belong to the final cell, and half 0 owns the diagonal.
An owner that is a hole or exceeds slope is rejected even if an adjacent triangle
is walkable. There is no nearest-point snap or neighbor fallback.

Dijkstra orders its heap by `(cost, triangle key)` and expands sorted neighbor
keys. Equal costs retain the first discovered predecessor. The edge metric is
ceil raw 3D Euclidean distance between each triangle's floor-quantized arithmetic
centroid. `NavigationPath::cost` is this graph metric, not waypoint polyline length.
At most `SearchBudget::max_expansions` nodes are expanded; exhausted budgets and
unreachable destinations are separate errors. No partial-success path is returned.

A complete path contains triangle keys, full-edge portals and start / portal
midpoint / goal waypoints. It deliberately does not claim funnel/string-pulling,
minimum Euclidean routes, or arbitrary obstacle clearance. Every quantized segment
is checked inside its designated closed convex triangle. Endpoint canonical owners
must be walkable; if a whole segment lies on an edge, that edge's interior canonical
owner must also be walkable. This prevents hole-owned seam travel despite walkable
vertices at its ends. Convexity proves the full open segment, not sampled points.

## Canonical persistence and stale dependencies

The little-endian format has no padding or trailing bytes:

1. Eight bytes `ORRNAV`, version 1, reserved 0
2. 32-byte terrain revision (SHA-256 of canonical terrain including asset identity)
3. u16 identity byte count and the terrain asset identity bytes
4. Four i64 profile raw fields: maximum slope, radius, headroom, maximum step
5. u32 vertex count followed by X/Y/Z i64 raw values for every canonical vertex
6. u32 accepted triangle count, followed by records in increasing stable key order
7. Each triangle: u32 key, three u32 vertices, u8 portal count; each sorted portal:
   u32 neighbor key, two sorted u32 shared-edge vertices

Graph revision is SHA-256 of these complete canonical bytes. Maximum input size is
2,200,000 bytes. Loading binds the supplied terrain identity/revision and profile,
rebuilds the bounded graph from that validated dependency, and compares every byte.
Untrusted counts cannot cause allocations. Malformed, reordered, alternate, stale,
truncated, oversized and trailing records fail, including forged connectivity.
Derived portal midpoint records are reconstructed, not trusted from storage.

Graphs and path bindings are private immutable data. A different terrain revision,
asset identity, graph or profile fails closed. Editing terrain requires an explicit
rebuild and replan; no automatic reroute can silently replace a user's route.

## Deterministic movement

`Navigator::advance` accepts a nonnegative FP distance budget, not wall time.
It charges the ceiling Euclidean 3D distance in raw units of each sampled endpoint
step, follows waypoint order, samples terrain elevation and never overshoots.
For a partial step, four floor/ceil XZ lattice candidates are considered. The whole
current-to-candidate and candidate-to-waypoint segments must remain valid. Among
safe budget-fitting candidates that reduce target distance, selection maximizes
forward dot product, then minimizes off-line error, then chooses smaller X/Z.
Tiny budgets may result in zero progress; callers must choose a useful fixed tick
budget (at least two raw units for diagonal movement). This quantized movement
contract measures sampled FP endpoints, not unrounded real-number arc length.

Status is Stopped, Moving or Arrived. Stop clears the path and freezes position.
Large budgets arrive exactly at the requested goal. Stale checks happen even for
zero budgets and stopped/arrived agents. Negative budgets fail. Advances and replans
are atomic: every error preserves all position, status, path and dependency state.
Explicit replan on fresh terrain resamples the agent's current XZ and commits only
when a complete route is available. `Navigator` owns heap vectors and is not an ECS
Pod; clone/snapshot it explicitly for replay. `checksum()` uses domain-tagged
SHA-256 over the complete canonical replay state with u32 counts/indices, excluding
pointers and vector capacities. This is not a state persistence/loader format. Use
an application-owned command
adapter. Automatic ECS/physics/collision integration is outside this slice.

## Tests

The crate contracts cover stable IDs, exact slope boundaries, reciprocal full-edge
portals, corner islands, canonical projection and holes, odd/tiny/extreme numerics,
canonical cook/load and every truncation/mutation of a small fixture, pinned
canonical state checksum and per-tick replay hash comparisons, stable bounded
Dijkstra, sampled elevation/distance budgets, deterministic state replay, stop and
arrival, explicit stale failure and atomic replan. Independent oracle tests compare
graph connectivity, search costs and every raw-quantized movement segment.
