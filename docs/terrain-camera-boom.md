# Terrain-aware camera boom (optional library)

`orr_terrain_camera` is a GPU-free presentation library for a camera following
one immutable terrain. It is not a physics controller. No consumer is switched
to it in this slice; Terrain Trail adoption, near-plane sizing, rendering and
native-window acceptance are separate work.

## Use and failure handling

Add the local path dependency only to the presentation consumer that needs it.
The crate depends on the existing public `orr_terrain` and `orr_terrain_view`
APIs, adds no registry dependency, and has no GPU or window feature or binary.

```rust,ignore
let obstacles = TerrainCameraObstacles::new(&terrain)?;
let result = obstacles.resolve(BoomQuery {
    expected_revision: render_terrain_revision,
    pivot: desired_pivot,
    desired_eye,
    radius: conservative_near_plane_radius,
    margin: 0.02,
    min_distance: 0.1,
})?;
match result {
    BoomResolution::Clear(pose) => draw(pose),
    BoomResolution::Retracted { pose, .. } => draw(pose),
    BoomResolution::Blocked { .. } => show_no_usable_camera(),
}
```

Keep desired orbit angles/zoom/distance independently from the returned pose.
Retraction is immediate and stateless; a clear next query returns the desired
eye unchanged. Do not reuse a previously successful pose after an error or
`Blocked`, force a zero-length camera, or clamp the blocked distance through the
obstacle. There is deliberately no smoothing or temporal collision promise.

The caller must enclose the actual near-plane rectangle (including aspect/FOV)
in its probe. A fixed tiny radius or a center ray is insufficient. Inputs and
output are **f64**. Narrowing the returned eye to f32 can move it by approximately
0.0625 per component near a coordinate of one million, much more than the
library's backoff. A consumer must account for that quantization separately;
this library does not certify an arbitrarily narrowed render pose.

## Snapshot identity and display agreement

Construction calls `orr_terrain_view::to_static_model(terrain, &[])`. It stores
the emitted vertex positions, without making an independent approximate FP
conversion. The bridge's exact-f32/Q16 precision and coordinate admission apply.
It checks the source identity/revision, single marker-free primitive, identity
transform, sequential indices, canonical triangle counts/order and exact FP
correspondence. Changed assumptions fail admission rather than silently changing
obstacle geometry. There is no `orr_model` direct dependency.

Each triangle has a stable `(cell [x,z], triangle 0 or 1)` ID. The diagonal is
the core's `0=[a,c,d]`, `1=[a,d,b]`; holes do not renumber neighboring cells. The
snapshot captures the source revision and requires that revision on every query.
Construct a new snapshot when terrain/render content changes. `triangle()`
exposes copied positions/identity for inspection, never mutable geometry.

An all-hole mesh is explicitly empty, including when unused source coordinates
would not be renderable. Otherwise horizontal spacing must be at least 1/256.
This additional conditioning limit is narrower than the display bridge alone.
Construction can allocate bounded model/snapshot data; queries do not allocate.
Neither construction nor querying changes source cook bytes or simulation state.

## Admitted queries and result semantics

All fields must be finite:

| Quantity | Inclusive range |
|---|---|
| Each pivot/desired-eye coordinate | -1,000,000 to 1,000,000 |
| Desired boom length | 0.0001 to 1,000,000 |
| Probe radius | 0.0001 to 10,000 |
| Caller collision margin | 0 to 10,000 |
| Minimum usable distance | 0.0001 to 1,000,000 |

Effective sweep radius is `radius + margin + NUMERIC_PAD`, with
`NUMERIC_PAD=1e-5`. The arithmetic envelope is another `1e-6` per boundary.
Returned retracted distance is first conservative contact minus
`CONTACT_BACKOFF=1e-5`, bounded below by zero. The hit reports contact before
backoff, with the stable triangle identity.

- `Clear` gives the unchanged desired f64 eye and finite boom distance
- `Retracted` gives a finite usable eye/distance and contact identity
- `Blocked::StartOverlap` means the start probe touches/overlaps the conservative
  obstacle; no camera pose is returned
- `Blocked::InsufficientDistance` means no admitted usable distance exists,
  including a minimum greater than desired length; no forced extension occurs
- Nonfinite/out-of-bounds queries, stale revision, unsupported display geometry
  and uncertifiable numeric arithmetic return explicit errors

Tangency counts as contact. Triangles are **two-sided rendered surfaces**, not a
solid half-space beneath the terrain. A camera already far below a surface may
be clear. Hole rims and outer edges still collide with a finite-radius probe.
This does not promise arbitrary scene collision or a visible goal from all angles.

## Continuous, conservative geometry

For each triangle, resolution considers the face prism, three finite edge
cylinders and three vertex balls over the entire segment parameter `[0,1]`.
The face/edge bounds are continuous linear interval clips. Tiny nonzero slopes
are not discarded. Each radial feature evaluates the convex distance minimum
over its finite clipped interval; when contact exists, at most 48 monotone binary
root refinements return the outside bracket. Refinement follows a proof of continuous
contact. It is not discrete motion sampling or a center-ray substitute. An
optional quadratic entry estimate is only an untrusted hint: a narrowed pair is
used only when checked finite endpoints inside the already-proven interval are
ordered, lower-outside and upper-inside. Otherwise the full bracket is retained.
Discriminant cancellation never rejects a contact. Refinement may stop early
only when the parameter width is <=2^-48, preserving the same spatial accuracy.

Plane, face-side and axial bounds and radial radii receive a `1e-6` arithmetic
envelope. Finite but underflow-scale radial motion uses a conservative
triangle-inequality bound instead of dividing by tiny squared speed. Nonfinite
intermediates fail explicitly. The first equal **computed f64** root retains
canonical row-major cell/triangle order. Near-equal mathematical roots need not
have the same ID across floating-point platforms. Each subsequent feature is
clipped to the already conservative nearest bound, while retaining t=0. A later
contact cannot invalidate that shorter pose. Hit identity refers to the retained
conservative bound, not necessarily the exact nearest geometric triangle within
arithmetic/refinement uncertainty.

**Conservative false positives are allowed.** Independently expanding side
planes of very acute triangles can overreach much farther than `1e-6`. There
is no claim of an exact Minkowski shape or a globally `R+1e-6`-close contact
distance. This is a bounded camera safety policy, not exact-real general-mesh CCD.

### Numeric audit

For the admitted exact-f32/Q16 coordinates, differences require at most 37 bits
and are exact in f64. Factoring grid spacing out of the terrain normals gives
`[yc-yd,s,ya-yc]` or `[ya-yb,s,yb-yd]`, avoiding cancellation from nearly parallel
long edge cross products. Scaled normalization bounds normal/edge-unit component
error by `8u`, and normalized inward cross components by `64u`, `u=2^-53`.
Construction verifies finite bases and plane/edge residuals.

The absolute sum of a point offset is at most `6e6`; displacement sum is at most
`sqrt(3)*1e6`. The conservative operand bound is `8e6`. Plane/side/axial evaluation
is bounded by `128u*8e6 <1.14e-7` world units; perpendicular projection, clipped
minimum and norm by `512u*8e6 <4.55e-7`. For an interior quadratic minimum,
multiplying parameter error by speed cancels the small-speed denominator;
underflow-scale squared speed `<=1e-24` takes the separate no-division branch.
Clip endpoints are expanded by `8*f64::EPSILON` in parameter space. The maximum
48-step bracket width is `1e6/2^48 <4e-9` world units, and final eye interpolation
rounding is below `4u*4e6 <1.8e-9`. The arithmetic envelope dominates these bounds,
in addition to the separate numeric inflation and path backoff.

## Bounded work, acceptance and limits

There are at most 32,768 canonical triangles (`MAX_SIDE=129`). Queries visit at
most that many triangles, each with one face plus three edge plus three vertex
tests and at most `6*48` radial refinements. The static whole-query ceilings are
229,376 feature tests and 9,437,184 refinements, plus at most two hint endpoint
probes per radial feature (393,216 total). The actual `bracket_probes` counter
reports this additional bounded work; `brackets_narrowed` counts certified
strictly smaller brackets. There is no allocation or unbounded
iteration in a query. `resolve_with_work()` returns actual work
counters; `storage_bytes()` reports retained snapshot allocation excluding
allocator bookkeeping, required to remain below 16 MiB.

The predeclared release-mode environment-local performance budget is <=1 second
for full 129x129 construction and, for the frozen 128-query max-mesh fixture,
p95<=8 ms and maximum<=25 ms. p95 is sorted nearest-rank `ceil(.95*N)-1`.
All samples and achieved work counts must be retained; a maximum-size mesh alone
does not imply every query reached the static refinement ceiling. These are
acceptance targets, **not measured results or universal latency guarantees**.
Failure prevents an interactive-performance claim; budgets cannot be relaxed
after seeing measurements.

Independent public-API tests specify plane, edge, vertex and tangent roots,
near-parallel crossings from both sides, thin ridge/long boom, hole/outer bounds,
revision/immutable bytes, overlap/minimum distance, numeric bounds, display
agreement and allocation/work caps. Source review, bounded execution and strict
checks are required separately. Exact-head required CI stays mandatory before
development integration; default CPU checks do not substitute for consumer GPU,
near-plane, orbit/zoom/reset or native-window proof. Main and PR1 are excluded.
