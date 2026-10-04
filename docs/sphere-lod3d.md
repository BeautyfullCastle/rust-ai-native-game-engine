# Optional sphere main-pass LOD

`Renderer3D::new` and `with_settings` retain fixed-detail rendering. To opt in,
use `Renderer3D::with_sphere_lod(rhi, format, settings, policy)`. The constructor
returns `SphereLodError3D` before allocating resources for an invalid policy.
`WindowRenderer3D::from_parts_with_sphere_lod` offers the same opt-in surface path.
Existing `Settings3D`, `Instance3D`, `RenderList3D` and `FrameStats` layouts remain
unchanged. No simulation, shader, other mesh kind or lighting rule changes.

| Near settings | Far policy | Cutoff |
| --- | --- | --- |
| `Settings3D::default()` (32 segments) | `SphereLod3D::default()` (12) | 6 physical target pixels |
| `Settings3D::LOW` (12 segments) | `SphereLod3D::LOW` (6) | 6 physical target pixels |

These are detail/quality candidates, not measured speed improvements. The far
segment count must be 3 through the effective near count (`mesh_segments`
clamped to 3..=64). The cutoff must be finite and positive. Equal near/far detail
builds no extra mesh and bypasses classification/packing.

## Classification and ownership

Every draw uses the current camera and physical target dimensions. The unit
sphere mesh is enclosed in a transformed local cube. Its scaled axes use the
same quaternion transform as the vertex shader; summing their absolute world
components gives a conservative world AABB, including nonuniform scale and
nearly-unit quaternion rounding. The maximum screen distance from projected
center to its eight projected corners is the bound radius. This overestimates
the visible sphere; it is not the exact screen-space circle radius.

Finite valid bounds at or below the cutoff go far. A zero viewport, invalid
camera/projection/view basis, nonfinite instance field, nonpositive scale,
quaternion outside the accepted squared-norm tolerance of `1e-3`, overflowed
projection, nonpositive clip `w`, or an AABB
corner on/across the near plane keeps original detail. This fallback concerns
LOD selection; it does not make malformed render data or zero-sized GPU targets
valid. No culling, hysteresis or cross-frame classification cache is added.

The source `RenderList3D` is read only. A retained instance vector and retained
classification bit vector pack near instances first and far instances second,
preserving every instance byte and relative order within each bucket. The
combined sphere slice is uploaded once to the existing instance buffer. Other
kind slices and debug lines retain their existing uploads. The main pass draws
each nonempty sphere bucket once. Shadows draw the entire combined sphere slice
once with the original near-detail mesh. Planes remain excluded from shadow
draws, and empty/shadows-off frames retain the existing shadow-map clear pass.

The far mesh is appended to the static mesh buffers with its indices rebased to
absolute vertex indices. Both buckets use a zero base vertex, including WebGL2,
whose indexed instanced draw path does not support a nonzero base vertex. A
mixed frame still uses a nonzero first instance for the far bucket; that offset
is separate from the mesh's base vertex.

## Counters

`last_sphere_lod_stats()` returns separate sphere-only `SphereLodStats3D`:

- Near/far/fallback counts, main/shadow sphere draw calls, sphere upload calls
  and payload bytes describe the last submitted draw.
- Index invocations are index count multiplied by instance count, not visible
  triangles, GPU work duration, FPS or an improvement ratio.
- `additional_static_mesh_bytes` is added vertex/index payload allocated and
  uploaded once at construction; it excludes allocator/driver overhead. The
  same two mesh buffer writes include this payload; frame upload counters do
  not include construction.
- CPU staging capacities are retained element capacities; classification
  capacity is in `Vec<bool>` bits. `staging_reallocations` counts capacity-growth
  events this draw. Existing `FrameStats.buffer_reallocations` still counts GPU
  buffer growth, so these counters have different scopes.
- `cpu_classify_time` includes classification and packing on native builds.
  It is unavailable on wasm and is also included in existing prepare time;
  adding those durations would double count it. CPU call return is not GPU
  completion. Before the first draw, counts/timing are empty and static payload
  metadata is available.

A sphere-only frame with N nonempty instances still has three frame uploads:
two 256-byte globals and one 80*N-byte sphere slice. Near sphere index counts
are 4,608 at default and 576 at LOW; far counts are 576 and 144 respectively.
Shadow counts remain those of the near mesh. Mixed buckets can increase sphere
main draw calls from one to two. Warm bucket changes at unchanged total count
reuse staging and GPU capacity; reduced count retains the previous capacity.

## Correctness fixtures and evidence boundaries

CPU tests cover policy validity, exact cutoff equality, current camera/resize,
invalid and near-plane fallback, rotated nonuniform bounds, stable byte/order
preservation, retained capacities, and equal-detail bypass. Native GPU fixtures
compare disabled/all-near images exactly on the same adapter and preset, check
far silhouette displacement <=1 physical pixel and center-channel difference
<=4 levels, and cover mixed material/depth, counters, resize, empty frames and
fixed-detail shadow receivers. The small-sphere scenes exercise these concrete
candidates; passing them is not a universal bound for arbitrary materials.

`ORR_REQUIRE_GPU=1` makes an unavailable native adapter fail. New LOD fixtures
accept `ORR_SPHERE_LOD_GPU_MODE=hardware|software` and report actual adapter,
backend, device type and driver; mode requests alone do not prove a backend.
Existing ignored performance baselines are outside this correctness phase.

The isolated `sphere_lod.html` / `tools/webtransport/sphere_lod.cjs` fixture
requests explicit WebGPU or WebGL2 and checks the returned backend. Its browser
test compares actual canvas PNGs for default and LOW near/far/mixed controls.
Mixed controls submit the far material first and the near material second, so
stable bucket packing must preserve both materials while drawing the far bucket
with a nonzero first instance. Disabled/all-near controls require identical RGBA
pixels. Far silhouettes use bidirectional distance of at most one physical pixel
(`dx*dx + dy*dy <= 1`); center RGB channels may differ by at most four levels.
Existing 2D browser tests do not prove this 3D path. Missing compiler/browser prerequisites
or unavailable adapters must be recorded as unverified; optional skips are not
acceptance evidence. Linux software Vulkan, Windows hardware Vulkan, Windows
WARP/DX12, WebGPU and WebGL2 results are distinct lanes.

This implementation phase adds correctness capability. Actual results and
source identity are recorded in the issue/PR handoff and local ignored evidence
directory. It starts no performance campaign. Before measuring improvement,
fix the baseline/current source, scene, commands, target, profile, features,
adapter, cache state, repeats and quiet execution plan with the owner.
