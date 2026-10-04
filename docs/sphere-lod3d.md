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

## Authorized CPU return observations (2026-10-04)

After the correctness implementation above, the owner approved one release
compilation and six serial observations of an observer-only test patch.
These observations do not establish a CPU improvement or measure GPU time.
Fixed-detail rendering remains the default; no production policy or renderer
default changed for this campaign.

### Measured source and reproduction conditions

The measured worktree was `codex/haneul-48-lod-perf-observer`, at parent
`18cbdfadfca01050192cd61a48cfe91662a996f3` (tree
`88d77983bd7bb4521a7c966165753d7433240e52`) plus the then-uncommitted
observer additions in `crates/orr_render/tests/gpu3d.rs`. The actual source
file was 64,193 bytes, SHA256
`138b01195a0265c2631ed48d6a2a9c7806822205a5db64e51ad5164d595b7ff1`.
The publication commit records these same executable-source bytes and this
later result section; its commit identity is not the measured parent or an
additional observation. The original PR50 correctness source is separate.

Windows 11 Education build 26200, Intel i7-14700 (28 logical processors),
Balanced power plan, Rust/Cargo 1.97.1 `x86_64-pc-windows-msvc`, Python 3.13.7.
The adapter reported **NVIDIA GeForce RTX 5070**, Vulkan, DiscreteGpu, vendor
4318/device 12036, NVIDIA driver 596.49, software=false. The observer requires
this exact adapter name, a discrete NVIDIA Vulkan adapter and actual 4x MSAA
before timing. Existing Cargo dependency/build caches were reused through
`CARGO_TARGET_DIR=E:/Projects/Fork/rust-ai-native-game-engine/target`.
Desktop/background services remained active (the prelaunch CPU snapshot was
36%); this is not whole-CPU isolation or a cold-system benchmark.

The scene is a 40x25 grid of 1,000 spheres. Radii 0.30/0.45/0.70/0.96 each
occur 250 times, assigned by `(x*7+z*13)%4`; positions are
`[x-19.5, 0.35, z-12]`. Camera eye `[0,28,34]`, target `[0,0,0]`, 55-degree
FOV, 640x360 `Rgba8UnormSrgb`, black clear, shadows on, default 4x MSAA and
2048 shadow map. Near/far detail is 32/12 segments with a 6-pixel cutoff.
The exact selectors construct the same scene for LOD off and on.

One compilation selected the executable from Cargo JSON:

```text
cargo test --locked --release -p orr_render --test gpu3d --no-run --message-format=json
```

It exited 0 in 36.500567 seconds. The release test profile recorded opt-level
3, debuginfo 1 and debug assertions disabled. The resulting executable was
7,312,896 bytes, SHA256
`88dd3bf5b4fea6ac478280d723e92aad85de5219564143b6aaaa621147a570ab`.
That same executable, with `ORR_REQUIRE_GPU=1` and
`ORR_BASELINE_GPU_MODE=hardware`, ran six fresh children in the approved
fixed order **off, on, on, off, off, on**. Each invocation used:

```text
<Cargo-JSON executable> --exact release_sphere_lod_perf_default_mixed_off --ignored --nocapture --test-threads=1
<Cargo-JSON executable> --exact release_sphere_lod_perf_default_mixed_on --ignored --nocapture --test-threads=1
```

Each child observed one first (cold) frame, ten warmup frames and thirty
steady frames, followed by one separately timed readback. There was no
external warmup, retry, extra build or extra observation. Each selector passed
once (1 passed, 0 failed, 0 ignored, 23 filtered). Actual child intervals were
10:44:47.489776 through 10:44:56.974488 UTC, serial with distinct PIDs.
Source/binary hashes were the same before/after each child and after capture.

### Six separate process results

All timings are **milliseconds**. Cold is the first CPU render-call return
observation in that child; steady columns give arithmetic mean / min / max
over exactly that child's thirty steady frames. No samples are pooled across
children. Readback is a separate final operation after the 41 frames.

| Order | LOD | PID | Cold CPU return | Steady CPU return (mean / min / max) | Final readback |
| --- | --- | ---: | ---: | --- | ---: |
| 1 | off | 47088 | 1.3599 | 0.187130 / 0.0421 / 0.5616 | 13.5761 |
| 2 | on | 20864 | 1.2458 | 0.279700 / 0.1613 / 0.6643 | 9.6435 |
| 3 | on | 44124 | 1.2704 | 0.261110 / 0.1724 / 0.7340 | 8.8119 |
| 4 | off | 43280 | 1.0271 | 0.172553 / 0.0419 / 0.6990 | 13.9393 |
| 5 | off | 7036 | 1.4905 | 0.323877 / 0.0467 / 1.7616 | 25.0172 |
| 6 | on | 31504 | 1.7220 | 0.568187 / 0.1694 / 3.7621 | 78.0786 |

CPU phase results below use the same per-child steady frames. Classify/pack/
staging is **inside prepare**; adding it to prepare would double count it.
It is null when LOD is off.

| Order | Prepare (mean / min / max) | Classify (mean / min / max) | Encode (mean / min / max) | Submit (mean / min / max) |
| --- | --- | --- | --- | --- |
| 1 | 0.058503 / 0.0126 / 0.2743 | null | 0.004067 / 0.0020 / 0.0106 | 0.124190 / 0.0260 / 0.3631 |
| 2 | 0.172300 / 0.1282 / 0.3389 | 0.127560 / 0.1128 / 0.2276 | 0.005073 / 0.0023 / 0.0136 | 0.101890 / 0.0302 / 0.3313 |
| 3 | 0.170377 / 0.1402 / 0.3141 | 0.129543 / 0.1266 / 0.1521 | 0.004073 / 0.0023 / 0.0108 | 0.086387 / 0.0296 / 0.4485 |
| 4 | 0.049597 / 0.0129 / 0.1885 | null | 0.006550 / 0.0020 / 0.0845 | 0.116080 / 0.0259 / 0.5246 |
| 5 | 0.089530 / 0.0130 / 0.4829 | null | 0.005663 / 0.0025 / 0.0147 | 0.228200 / 0.0299 / 1.5810 |
| 6 | 0.271190 / 0.1291 / 1.2001 | 0.132420 / 0.1154 / 0.1928 | 0.007300 / 0.0026 / 0.0252 | 0.289113 / 0.0375 / 2.5542 |

Cold prepare/classify/encode/submit values (ms) are retained separately:

| Order | Prepare | Classify | Encode | Submit |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.7433 | null | 0.0282 | 0.5850 |
| 2 | 0.7230 | 0.1317 | 0.0247 | 0.4956 |
| 3 | 0.7736 | 0.1388 | 0.0225 | 0.4719 |
| 4 | 0.6143 | null | 0.0200 | 0.3905 |
| 5 | 0.6712 | null | 0.0212 | 0.7952 |
| 6 | 0.9745 | 0.2247 | 0.0333 | 0.7118 |

| Per-frame submitted/capacity counter | LOD off | LOD on |
| --- | ---: | ---: |
| Near / far / fallback instances | 1000 / 0 / 0 | 643 / 357 / 0 |
| Main sphere indices / draws | 4,608,000 / 1 | 3,168,576 / 2 |
| Shadow sphere indices / draws | 4,608,000 / 1 | 4,608,000 / 1 |
| Frame upload bytes / calls | 80,512 / 3 | 80,512 / 3 |
| LOD portion upload bytes / calls | 80,000 / 1 | 80,000 / 1 |
| Additional static mesh payload bytes | 0 | 5,580 |
| Retained staging capacity (instance elements) | 0 | 1,000 |
| Retained classification capacity (boolean bits) | 0 | 1,000 |
| Cold attachment allocations / staging reallocations | 2 / 0 | 2 / 2 |
| Subsequent 40-frame allocation/reallocation counters | all zero | all zero |

All 41 frames per child recorded shape=0, mesh=1000 and line=0. The reduced
main index count is submitted logical work; it is not a GPU time or speedup.
Final readback was 921,600 RGBA bytes per child; recorded FNV1a64 was
`9577a303019a3288` with LOD off and `b5fb89a0c9ba9967` with LOD on. Visibility
assertions passed. The separate correctness image tests above remain their
own evidence; this campaign exported no new PNGs for direct visual review.

### Capture integrity and limits

There are 258 JSON records (six metadata, 246 frames, six readbacks). All
fourteen stdout/stderr streams, including the build, total 337,784 saved
bytes; observed/saved/file sizes and SHA256 values agree. Seven child
processes naturally exited 0; both streams reached EOF and drain threads
completed. Post-campaign census found no related live process and the quiet
lane was returned in issue comment 5979157327. No separate measured process-
handle-closed field exists, so process-handle closure is not an instrumented
result. Soft fail-stop time/output thresholds were not crossed; this campaign
does not exercise forced preemption or prove a hard deadline.

CPU render-call return is not GPU completion, GPU execution duration or FPS.
The final readback boundary is not a per-frame GPU timer. These six process
observations, in fixed rather than randomized order with background activity,
provide no CPU improvement conclusion, pooled ratio, median, p95 or product
budget. Static payload bytes and element/bit capacities exclude full heap,
allocator/driver overhead, RSS and private usage; none were sampled here.
The default remains off.

### Local immutable evidence

Raw logs, the exact executable copy and source pin snapshots remain local in
`E:/Projects/Fork/rust-ai-native-game-engine/target/haneul-48-lod-perf-actual-20261004T1042Z/`.
This directory label is not an API or measurement timestamp. The files record
actual UTC intervals and their own derived/read times separately. Root and
independent first-data/process/result-table reviews found no blocker.

| Local artifact | SHA256 |
| --- | --- |
| `capture-manifest.json` | `6bdf5b4c7aff09f98821166a950d430b18ac3c32503f656ba6f95713d692aa98` |
| `actual-six-process-summary.json` | `fdcf23e28160146d4a6382b863eb7538f1e3dc21fa6e6b3e82cc3809ca3c3a4a` |
| `actual-six-process-results.md` | `54ceed22ffec378c7e2b7bd5888da74f37daa308e2229639e01ab9ab6861dfa1` |
| `independent-actual-process-proof-1045Z.json` | `f7e74995657e14cd990e22eb4d4bfd8f9fc67a3649682eac33a130e2328d1761` |
| `independent-actual-observation-review-1046Z.json` | `04bbf6803343dfcff1104783d95f211c21de6681ce0b7c843e542c17589fed77` |
| `derived-results-narrow-review-1057Z.json` | `36c15c5b62784330ae44fe4454e80df17665d8c14897d62f34fcc6c67fa4faf5` |
| `immutable-local-copy-proof.json` | `dc7c0527f63cb31d1a3ac53f796d490b6c6f4e54485cc96581131529a8d24d98` |
| `first-actual-completion-proof.json` | `db6630f85d79fddaa2f62dc92e2dec60d73d5d175831cf1143ff8a2687087ae2` |

The two observer tests are ignored by ordinary CI. Publication CI validates
its own source/event head and may check a separate synthetic merge commit;
it does not replay or replace these six local observations. This observation
stage and result publication do not by themselves close the #48 epic.
