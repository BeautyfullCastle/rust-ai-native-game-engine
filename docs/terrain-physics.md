# Optional bounded sphere–terrain physics

This is the first **sphere-only, static-heightfield slice of #102**. The wider
terrain collision issue remains open. It does not implement general
box/capsule/convex terrain collision, a moving terrain mesh, editable collision
geometry during play, or continuous collision detection (CCD).

The optional `orr_terrain_physics3d` crate joins `orr_terrain`, `orr_ecs`, and
`orr_physics3d`. Core ECS does not depend on terrain. Existing convex physics and
ordinary Yard3D hosts retain their original entry points; opt in explicitly with
the `terrain-physics` feature and the `terrain-yard3d` game.

## Run the authored example

From the repository root:

```sh
cargo run -p orr_remote --features terrain-physics --bin orr_remote_host -- \
  --game terrain-yard3d --scene scenes/terrain_sphere.scene.yaml --dev-no-auth
```

The native editor uses the same host admission and simulation:

```sh
cargo run -p orr_editor --features terrain-physics -- \
  --game terrain-yard3d --scene scenes/terrain_sphere.scene.yaml
```

For local agent access to that editor's host, add
`--erp 127.0.0.1:7777 --erp-dev`. Use the ordinary `orr status`, `orr scene`,
`orr sim start`, and `orr sim step 120` workflow against that endpoint. Do not run
the standalone host and editor listener on the same port at the same time.
`--dev-no-auth`/`--erp-dev` are local development modes, not deployment defaults.

The fixture contains two radius-0.5 dynamic spheres: one falls onto a solid patch
and the other falls through a two-by-two-cell hole. The host runs at 60 Hz with
seed 42, build ID 0, and two idle input slots. It has no player-spawn system,
rain, walls, recycling bounds, or hidden floor proxy.

The checked-in pin resolves `scenes/terrain/sphere_demo.orrt`. Its canonical
SHA-256 is:

```text
def69009fd03724f628d6f8a2c4e1bd8da147450db183307ef0c2ffab238d465
```

These are launch instructions, not a claim of a successful native-window smoke
run. Headless integration tests, compilation, and offscreen rendering evidence
must be reported separately from actual interactive window verification.

## Authoritative state and admission

The complete collision asset belongs to the `Frame`:

- `TerrainAsset` is a registered Pod singleton with the full 32-byte SHA-256,
  width/depth, world X/Z origin, spacing, collider entity, and list handles
- Distinct registered `FrameList<TerrainHeight>`, `FrameList<TerrainHole>`, and
  `FrameList<TerrainIdentityByte>` pools hold every row-major height, every
  canonical 0/1 hole flag, and every identity byte
- A real entity carries `HeightfieldCollider`: full revision reference,
  friction, restitution, layer, and mask. It deliberately has no convex `Body`
  or `Collider`; attaching either is an invalid ambiguous terrain object
- The normal physics `PhysicsState`/`FrameList<ContactCache>` owns persistent
  warm-start impulses and sleep-relevant contact references

Consequently `Frame::checksum`, cloning/`copy_from`, and ORRF byte snapshots cover
all inputs that can affect terrain collision, including identity and hole data.
There is no authoritative pointer-keyed cache, editor document, or filesystem
lookup during a simulation step. Reconstructing a `Terrain` or a render mesh from
these lists produces a derived read-only value.

`asset::register` registers terrain types without registering core physics.
Call `orr_physics3d::register` separately. `admit_heightfield` accepts canonical
bytes, an expected **full** revision, and explicit material/filter state. It
validates format, canonical encoding, all revision bytes, and the collision
profile before allocating any lists or entity. Every returned error leaves the
entire frame unchanged. A second admission is rejected, including an otherwise
identical asset; this slice has no in-place replacement API.

`terrain_view` validates the frame's references, live collider, content lengths,
canonical identity/hole encoding, numeric profile, material, and SHA-256 before
borrowing geometry. The canonical digest is recomputed from frame content, not
trusted because a snapshot's ordinary checksum happened to pass. Snapshot
checksums establish byte integrity, not semantic validity of arbitrary Pod
values. `reconstruct_terrain` offers an allocating read-only reconstruction for
consumers that need the existing terrain API.

### Scene and editor lifecycle

`TerrainScenePin` is authored scene data: a length-delimited source path and
asset identity, all 32 revision bytes, and material/filter values. Its Pod byte
arrays have zero tails. The source path is host admission metadata, not runtime
asset authority.

`LocalTerrainAdmission` fixes a root at the original scene's parent directory.
Every initial load and rebake reads a bounded regular `.orrt` file under that
root and verifies both the complete identity and full revision. Absolute asset
paths, parent traversal, empty or dot segments, backslashes, colons, symlink path
components, and `.orr` path segments are rejected. ERP-supplied save paths cannot
expand this authority root. Source contents changed without an updated pin are
refused on the next bake; deleting the source after successful admission does
not remove collision geometry from an already-running frame.

The generic fallible `BakeAdmission` hook runs on an isolated candidate after
ordinary scene baking. Field and singleton edits cannot bypass it through the
fast patch path. Loads, rebakes, structural edits, undo/redo, proposal staging,
and proposal acceptance use the same admission path. A failed candidate keeps
the live scene, frame, GUID allocation state, revision, and history intact.
Proposal acceptance stages the whole operation list, so a successful prefix
cannot leak if a later asset read fails. Open admitted transactions retain a
pre-transaction frame/scene snapshot; rollback restores it without needing the
external asset to still exist.

Ordinary save of the authored document is supported. Opening a different path,
Save As, live play edits, raw play debug mutation, and play-frame scene capture
are disabled in this first terrain host. Capture is especially unsafe without a
policy for excluding derived runtime entities from a new authored scene. Stop
play before editing. To change collision content, update the cooked asset and
its complete pin, then admit a fresh baked frame; this is not live collision
painting.

The editor's terrain presentation is reconstructed from admitted frame lists
through the read-only `FrameView`. Its source is the same content used by
physics; a separate editor terrain document cannot silently become simulation
authority.

## Geometry and contact semantics

Coordinates are Y-up, with the heightfield laid out in world X/Z. For each cell,
`a` is its minimum-X/minimum-Z vertex, `b` is +X, `c` is +Z, and `d` is the
opposite corner. Collision uses the canonical triangles `[a,c,d]` and `[a,d,b]`,
matching `Terrain::triangles`, vertex positions, and the piecewise-planar
`sample`/`surface` APIs. It does not substitute bilinear interpolation.

The narrow phase computes the closest point on each **finite** candidate
triangle using checked integer intermediates. A contact may belong to a face,
finite edge, or vertex. Face normals point toward the sphere, so the surface is
two-sided. Outer rims and hole boundaries use their actual finite-edge/vertex
normal, rather than an invented vertical wall. A sphere can pass under the
heightfield and encounter its underside.

A hole removes both triangles of its cell. There is no bottom surface, hole
wall, infinite plane, or implicit collision outside the terrain boundary. A
sphere still touches a neighboring solid rim if its radius reaches that rim.
This geometric reach differs from the point-sampling API's ownership rule at
cell seams; collision considers the finite neighboring triangles themselves.

Mesh-wide feature keys distinguish vertices, edges, and faces. Shared features
are deduplicated deterministically. Incident-triangle tangent tests reject an
internal edge/vertex that is not a local distance minimum, preventing flat
triangle diagonals and cell seams from acting as spurious obstacles. Real ridge
or rim contacts remain. Each constraint retains its own normal, including
multiple normals for one body–terrain pair.

The optional `step_with_static_contacts` solver seam converts the admitted
collider into an immovable scratch contact object. Contacts refresh from current
poses before every solver substep and once after final integration for
validation and final cache/sleep ownership. Removed support cannot leave a stale
endpoint cache or put a sphere to sleep above a hole. It uses the ordinary material mixing, filtering, friction,
restitution, warm starting, sleeping, impulse wake-up, and integration machinery.
Scratch is disposable and safe to rebuild after rollback. It does not carry
persistent terrain samples or persistent contact authority.

The provider path uses checked wide intermediates for constraint preparation,
velocity targets, warm starting, impulse sweeps and friction-cache storage.
An unrepresentable narrow solver intermediate returns `NumericOverflow` before
frame publication. This protects over-constrained or malformed states beyond the
ordinary admission limits; it never silently wraps release arithmetic. The
original convex-only fast solver and its checksum goldens are unchanged.

Support loss is tracked across the entire substep sequence, including incoming
cached support, and is cleared only by reacquired endpoint support. Under nonzero
gravity, the optional path sleeps only islands connected through positive-normal-
impulse endpoint contacts to immovable support whose normal opposes gravity.
Terrain contacts must be within `linear_slop`; ordinary convex contacts retain
their existing `contact_margin` collision skin. A disconnected falling cluster,
far speculative terrain proximity, or sideways/ceiling contact cannot ground an
island. Friction-only wall support conservatively stays awake. Zero-gravity free
sleep and the original no-provider sleep behavior remain unchanged.

Already-sleeping restored frames receive the same support check before the idle
shortcut. Read-only temporary scratch enables all dynamic bodies only for
contact discovery, rebuilds current ordinary and terrain contacts, matches full
pair/feature cache keys, and derives connectivity without trusting saved island
labels. Unsupported sleepers return an atomic error; legitimate grounded stacks
remain byte-identical. Explicit and orphan waking are preserved. The callback
may therefore receive an additional read-only validation call containing
sleepers. This bounded validation is repeated at public adapter boundaries;
no performance improvement is claimed.

## Explicit numerical profile

All simulation quantities use Q48.16 `FP`; one raw unit is 1/65536 world unit.
The collision profile is intentionally narrower than the standalone asset
format. Bounds are inclusive except where marked otherwise.

### Asset profile

| Quantity | Limit |
| --- | --- |
| Vertices along each axis | 2–65, at most 4096 cells |
| Origin, far X/Z edge, and every height | −1024 to +1024 |
| Cell spacing | 0.25–16 |
| Height difference across an axis-adjacent edge | At most 16 |
| Identity | Canonical `orr_terrain` identity, 1–256 bytes |
| Admitted terrain assets | Exactly one for the terrain host |
| Terrain transform | Static, world-aligned; no attached convex body/shape |

The asset API accepts friction 0–100 as a stored collider material. The bounded
runtime profile tightens terrain friction to 0–2, matching ordinary colliders;
the host validates that runtime profile before publication.

### Runtime profile

| Quantity | Limit |
| --- | --- |
| Ordinary physics bodies | At most 64; each Body has exactly one Collider |
| Dynamic terrain-contact shapes | Spheres only; radius 0.25–8 |
| Body center coordinates | Each axis −1024 to +1024 |
| Dynamic inverse mass | 0.25–64 |
| Principal inverse inertia | Each axis 0–1024 |
| Linear/angular damping | 0–64 |
| Linear speed cap | Greater than 0, at most 32 per axis |
| Angular speed cap | 0–64 per axis |
| Gravity | Each axis −128 to +128 |
| Collider friction / restitution | 0–2 / 0–1 |
| Collider flags and explicit padding | Zero |
| Quaternion components | −1 to +1; squared norm within 1/256 of one |
| Tick duration | Greater than 0, at most 1/30 second |
| Substeps / velocity iterations | 1–64 / 1–16 |
| Substep duration | At least one raw fixed-point time unit |
| Baumgarte / linear slop | 0–1 / 0–0.0625 |
| Contact margin | 0–0.25 |
| Restitution threshold / correction speed | 0–128 / 0–16 |
| Sleep linear/angular speed / sleep ticks | 0–1 / 0–65535 |
| Candidate triangles per sphere refresh | At most 4096, counting hole cells |
| Retained contacts per sphere refresh | At most 64 |
| Persistent cache entries | At most 16384; sorted unique live-entity keys |
| Cached normal / per-axis tangent impulse | 0–1024 / −1024 to +1024 |

These independent bounds are necessary but not sufficient: the displacement
and penetration checks below must also pass. Large spheres over finely spaced
terrain can exceed the candidate budget even when both inputs individually fit
the table; that step is explicitly refused.

`terrain_config()` uses the ordinary 60 Hz/eight-substep configuration with
linear and angular caps of 16. It satisfies the displacement test for radius
0.5 or larger. A radius-0.25 sphere needs a lower speed cap or more substeps.
Do not reuse ordinary physics' default speed cap of 500 for terrain.

At the crate adapter boundary, a non-sphere body whose two-way collision filter
excludes terrain can continue using ordinary convex physics within the numeric
profile. A filter-permitted non-sphere, static sphere, or kinematic sphere is
rejected even when it is asleep or far away. The `TerrainYard3D` host is stricter:
all authored physics objects must be dynamic spheres, regardless of filtering.

## Discrete motion guard; no CCD claim

The solver refreshes contacts every substep and clamps post-solve linear
velocity before integration. For raw per-axis speed cap `V_raw` and maximum raw
substep duration `h_raw = ceil(dt_raw / substeps)`, the adapter bounds the
substep's L1 displacement by:

```text
D_raw = ceil(3 * V_raw * h_raw / 65536) + 3
D <= radius / 4
```

The extra three raw units conservatively cover coordinate quantization. At the
initial boundary, every substep boundary, and the final integrated boundary,
the nearest finite-mesh distance must be at least `radius / 2 + 8 raw units`.
The extra distance guard covers closest-point rounding. A center crossing a
finite triangle would require travel of at least approximately `radius / 2`
from the initial guarded state, while accepted substep displacement is at most
`radius / 4`. An invalid configuration, unsafe body state, deep starting overlap,
or failed final boundary check is rejected atomically.

This bounded no-center-crossing argument is **not swept CCD**. It does not find
the exact time of impact, promise first grazing-contact detection, allow
arbitrary speeds, or rescue deep initial penetration. Ordinary shallow overlap
is handled by the contact solver. A sphere that leaves the permitted world
range or violates another bound later can cause a future tick to be refused.

`terrain_step` leaves the entire frame unchanged on a returned error, including
errors detected after solving. The host then records a bounded diagnostic in
`TerrainStepStatus` and stops further physics; it never silently switches to
convex-only stepping or installs a fallback floor. That explicit failure status
is the host's intentional state change, separate from the rejected physics step.

## Verification and remaining scope

Focused verification commands:

```sh
cargo test -p orr_terrain_physics3d --test frame_assets
cargo test -p orr_terrain_physics3d --test terrain_physics
cargo test -p orr_physics3d --test static_contacts
cargo test -p orr_edit --test bake_admission
cargo check -p orr_remote --features terrain-physics --bin orr_remote_host
cargo check -p orr_editor --features terrain-physics
```

Asset tests cover full-revision rejection, malformed canonical bytes, profile
edges, byte-for-byte atomic failures, every authoritative checksum field,
snapshot reconstruction without the original asset object, and rejection of
semantically invalid restored Pod state. Geometry/runtime tests exercise actual
falling and resting spheres, slopes, friction, sleeping/wake-up, ridge/rim
response, seams, holes, finite boundaries, filtering, guarded high-speed
refusal, candidate budgets, and deterministic rollback with fresh scratch.
Admission tests cover history, proposals, GUID preservation, and asset loss
between operations. These tests do not substitute for measuring performance,
cross-target checksum testing, or interactive native-window verification.

Still outside this slice, and therefore **#102 remains open**:

- Terrain contacts for boxes, capsules, general convex hulls, or arbitrary meshes
- Swept CCD, unrestricted high-speed contact, and deep-overlap recovery
- Multiple terrain assets, streaming/chunking, rotated/scaled/moving terrain
- Live terrain deformation or replacement with explicit cache invalidation
- A general play-capture policy for derived asset entities
- A broader authored asset package/resolver lifecycle beyond the confined local
  scene-relative pin used by this host
