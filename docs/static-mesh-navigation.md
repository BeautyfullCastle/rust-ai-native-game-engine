# Static-triangle navigation workflow (partial #103)

`orr_navigation::mesh::StaticMeshGraph` provides deterministic, GPU-free navigation
over explicitly supplied fixed-point triangle surfaces. Unlike `TerrainGraph`, it
does not require a rectangular heightfield: a level can supply ramps, bridges and
separate floors at the same XZ coordinates. This is an optional navigation-crate
capability, not a new core dependency, editor tool, mesh importer or physics system.

## Bake, save, load, plan and follow

Run the complete ramp-to-bridge example from a Windows checkout:

```powershell
cargo run --release -p orr_navigation --example static_mesh -- --output "$PWD\bridge.orrmesh"
```

The example validates the in-memory asset and route, creates an absent destination
without overwriting an existing file, synchronizes its bytes, reloads the asset,
and compares fixed-tick distance samples before and after reload. Its trace ends
at the exact goal. `--help` documents the invocation. This is a CPU navigation
workflow, not evidence of an editor-controlled character or an exported game.

The library entry points are `StaticMeshGraph::bake`, `to_bytes`, `from_bytes`,
`project`, `find_path`, `validate_path`, `path_length` and `point_at_distance`.
Pass triangle vertices as `[[FP; 3]]`, faces as `[[u32; 3]]`, and the existing
`AgentProfile` and `SearchBudget` types. Returned paths contain their graph
revision, triangle corridor and start / portal / goal waypoints. Navigation
errors reuse `NavigationError`; `OutsideTerrain` also means no eligible mesh
surface at the requested XZ and vertical tolerance.

## Geometry and numerical contract

- Y-up, upward-facing triangles with nonzero XZ area; wall and downward triangles
  are rejected. Flat and sloped walkable surfaces may coexist.
- Point agents in open sky only. Radius, headroom and maximum step must be zero;
  nonzero values are rejected, not silently ignored.
- Maximum slope is a rise/run ratio from 0 through 4, inclusive, not an angle.
  Admission compares the exact squared plane gradient using bounded i128 math.
  Triangles exceeding the selected slope remain in canonical source geometry but
  are not traversable.
- At most 65,536 input vertices and 32,768 input faces. Each coordinate's raw
  magnitude is at most 2^40; each axis delta on every triangle edge is at most
  2^22 raw (64 world units). Larger surfaces need explicitly subdivided triangles.
- Coordinate welding is exact, never epsilon-based. Vertices sort by raw XYZ;
  faces rotate cyclically to their smallest vertex, preserve winding and sort
  lexicographically. Duplicate, welded-degenerate and non-manifold faces fail.
  Face keys include rejected slopes, so walkable keys may have gaps.
- Only complete, welded shared edges create reciprocal portals. Corner contact,
  near edges, gaps and T-junctions do not create connections. A portal lies on the
  actual 3D edge's integer lattice, using half of its primitive lattice steps;
  a primitive edge with no interior lattice point uses its canonical endpoint.

Projection preserves the supplied XZ and floors the exact plane elevation to a raw
FP value. It selects the smallest vertical difference within the explicit tolerance,
then the smallest triangle key. Separate overlapping floors are not connected
unless they share an actual complete edge. There is no nearest-XZ snapping,
ceiling clearance check or automatic obstacle avoidance.

## Route and movement contract

Dijkstra uses a stable `(cost, triangle key)` heap and sorted neighbor keys.
Every edge costs at least one raw unit, otherwise the ceiling Euclidean distance
between floor-quantized triangle centroids. Search is bounded by the supplied
expansion budget, with no partial-success path. This graph metric is not a
minimum-distance surface route; no funnel or string pulling is claimed.

`validate_path` checks the graph revision, corridor adjacency, canonical portal
waypoints and each segment's endpoints against its designated convex triangle.
The closed XZ segment therefore stays inside that triangle. Quantized Y follows
its plane; samples do not establish collider clearance.

`path_length` sums ceiling Euclidean lengths of the waypoint segments.
`point_at_distance` samples an **absolute distance along that polyline**, not a
per-tick movement budget. Negative distances fail; distances at or beyond the
end clamp to the goal. A partial sample quantizes coordinates, projects Y onto
the designated triangle and uses a bounded neighboring-lattice fallback when
rounding would otherwise leave it. Failure returns an error, not an off-mesh
position. Quantization can change the endpoint-to-endpoint distance by raw units;
this API does not promise an incremental speed cap or collision response.

Store the path and absolute distance in application-owned replay state. The mesh
API has no wall clock, randomness, ECS component, automatic replanning or physics
controller. A path from another graph fails with `StaleGraph`; callers must also
bind the baked geometry to their actual level revision before using it.

## Canonical asset format and trust

All fields are little-endian, with no padding or trailing bytes:

1. Eight bytes: `ORRMESH` followed by version byte 1.
2. Four i64 raw profile values: slope, radius, headroom, maximum step.
3. u32 canonical vertex count, then three i64 raw coordinates per vertex.
4. u32 canonical source face count, then three u32 vertex indices per face.

The file limit is 2,200,000 bytes. Loading checks counts and available bytes before
allocating, rebakes the graph and compares the complete canonical encoding.
Derived portals, connectivity and walkability are reconstructed, never trusted
from a file. Truncated, oversized, reordered and noncanonical encodings fail.
The revision is SHA-256 of the domain tag `orr-navigation-static-mesh-v1\0`
followed by the complete canonical asset bytes.

The format does **not** embed an expected digest, signature or external world
identity. A different valid canonical geometry is a different valid asset with a
different revision, not a detected tampering event. Package/content authentication
and binding to render or collision meshes remain the consumer's responsibility.
The example's create-new output is not a transactional multi-file export or a
crash-durability guarantee; a failed write may leave its newly created partial file.

## Remaining game-production work

Static triangles no longer need to be represented as a terrain grid merely to
plan a point-agent route. Full #103 and #94 completion still require clearance,
dynamic obstacles, character/physics integration, editor authoring and actual
gameplay acceptance. Neither this optional module nor its CPU trace establishes
the complete editor create / save / play / source-independent export workflow.
