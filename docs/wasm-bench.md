# Web and mobile performance (M6 step 4)

Measured first, then optimized where wasm was slow. Everything below is reproducible with
`tools/wasm_bench.sh` (native, wasm32-wasip1 under wasmtime 25, wasm32-unknown-unknown in headless
Chromium 141) and `cargo test -p orr_wasm_bench` (the pinned checksums). `docs/progress.md` and
`docs/design-v1.md` are not edited here; fold the conclusions in when you review.

Machine: the 4-core cloud box of the session (noisy: single runs differ by 10% or more, so the numbers
are the best of 3 runs, before and after run alternately).

## 1. The harness

`crates/orr_wasm_bench`: 12 cases, each builds its state once and then runs N iterations and returns a
checksum. Timing is outside the library (`bench_runner` with `Instant`, or `performance.now()` in
`web/index.html`), so the same code runs everywhere.

| Case | What |
|---|---|
| `fp_mul`, `fp_mul_small`, `fp_div`, `fp_sqrt`, `fp_sin_cos`, `fp_atan2`, `fp_vec3_normalize` | one operation per iteration on game-like data (magnitude 2^8..2^40 raw); `fp_mul_small` has operands that fit 32 bits |
| `ecs_integrate_100k`, `ecs_checksum_100k`, `ecs_copy_100k` | one pass over 100 000 entities with 3 components (ns per entity) |
| `physics2d_1000_tick` | `PhysGame`, 1000 bodies, mixer scene (never settles), one tick |
| `physics3d_1000_tick` | `Yard3D`, 1000 bodies raining, one tick |

`crates/orr_wasm_bench/tests/checksums.rs` pins the checksum of every case at a fixed iteration count. The
values were recorded **before** the optimizations and every target must reproduce them (see section 4).

## 2. Native versus wasm, before and after

ns per operation (ns per entity for ECS, microseconds per tick for physics), best of 3.

| Case | native before | native after | wasmtime before | wasmtime after | Chromium before | Chromium after |
|---|---|---|---|---|---|---|
| `fp_mul` | 0.90 | 0.88 | 5.97 | **1.91** (3.1x) | 4.10 | **1.80** (2.3x) |
| `fp_mul_small` | 1.01 | 0.99 | 5.98 | **1.54** (3.9x) | 4.18 | **1.58** (2.6x) |
| `fp_div` | 5.01 | **3.29** (1.5x) | 13.27 | **3.37** (3.9x) | 10.62 | **3.18** (3.3x) |
| `fp_sqrt` | 156.2 | **7.29** (21x) | 145.9 | **6.89** (21x) | 159.8 | **7.43** (21x) |
| `fp_sin_cos` | 11.68 | 10.95 | 12.72 | 12.48 | 11.79 | 12.37 |
| `fp_atan2` | 12.39 | 12.19 | 13.47 | 12.36 | 13.25 | 12.81 |
| `fp_vec3_normalize` | 191.3 | **19.6** (9.8x) | 223.9 | **19.9** (11x) | 216.9 | **19.7** (11x) |
| `ecs_integrate_100k` | 1.40 | 1.53 | 6.63 | 6.80 | 4.73 | 4.70 |
| `ecs_checksum_100k` | 4.38 | 4.50 | 13.21 | 13.13 | 8.77 | 9.08 |
| `ecs_copy_100k` | 6.52 | 6.54 | 6.76 | 6.70 | 6.45 | 6.46 |
| `physics2d_1000_tick` (us) | 594 | 617 | 1470 | **996** (1.48x) | 1262 | **883** (1.43x) |
| `physics3d_1000_tick` (us) | 3176 | 3032 | 4304 | 4442 | 4627 | 4069 (1.14x) |

Reading it:

* Before, a wasm `FP` multiply cost 5 to 6x native and a divide 2.6x, because wasm has no widening
  multiply or 128-bit divide: every `i128` product was a call to a software routine. The 2D physics tick was
  2.5x native. The bit-by-bit `u128` square root was slow everywhere (156 ns native).
* After, wasm multiply and divide are within 2x of native, `sqrt` is 7 ns everywhere, the 2D physics tick is
  1.4 to 1.6x native (was 2.1 to 2.5x), the 3D tick (already mostly `i64` code with a table square root) is
  1.3 to 1.4x native.
* What is left: ECS passes are 3 to 5x slower on wasm than native. Native auto-vectorizes the integrate loop
  with SSE2 and the checksum (xxh3) with SSE2/AVX2; wasm without `simd128` does neither. See section 3.
* `sin_cos`/`atan2` (tables) were already equal; the 3D and 2D tick numbers include them.

## 3. What was optimized (all bit-identical)

| Change | Where | Idea |
|---|---|---|
| `FP::mul` on wasm32 | `crates/orr_fp/src/fp.rs` | operands that fit 32 bits multiply in one `i64` op; otherwise the 128-bit product's low 64 bits of `>> 16` are built from four `i64` multiplies (`mul_raw_split`). Native keeps the single `i128` multiply (LLVM emits one `mul`). |
| `FP::div` (all targets) | same | `(a << 16) / b` in `i64` when `|a| < 2^47` (numerator and quotient fit); the `i128` division only beyond that. |
| `FP::sqrt` (all targets) | `crates/orr_fp/src/trig.rs` | below `2^48` raw (4096 units: nearly all values) a table guess, two Newton steps and a correction in `u64`; the 64-step `u128` loop only above. Same table as `orr_physics::fastmath`. |
| 2D physics `fastmath::mul` on wasm32 | `crates/orr_physics/src/fastmath.rs` | `i64::checked_mul` is a 128-bit multiply on wasm; use `FP`'s operator there. |
| Profiles | root `Cargo.toml` | `web` (opt-level 3, fat LTO, one codegen unit, `panic = "abort"`, no debug info) and `web-small` (opt-level `s`). |
| `simd128` (opt-in) | `WEB_SIMD=1 tools/build_web.sh` | build flag only, no code change. Chromium, best of 3: ECS checksum 9.35 -> 5.99 ns/entity (1.56x), 3D tick 1.23x, integrate 1.11x, but the raw `fp_mul` loop 0.57x and the 2D tick unchanged, so it is **off by default** (also: needs Safari 16.4+, Chrome 91+, Firefox 89+). |

### Proof that nothing changed

* New tests compare the fast paths with the plain `i128`/`u128` formulas of the crate-level contract on edge
  values and about 1.2 million random operands: `crates/orr_fp/tests/fast_paths.rs` (operators), the unit
  tests in `crates/orr_fp/src/fp.rs` (the wasm multiply path is compiled on every target, so native runs test
  it too), `sqrt` also against the bitwise definition for raw 0..300 000 and at every power of two.
* No golden value changed: `orr_fp` (`determinism_golden`), `orr_ecs`, `orr_session` arena goldens,
  `orr_physics` and `orr_physics3d` goldens, plus the new `orr_games` and `orr_wasm_bench` goldens, pass on
  x86_64, wasm32-wasip1 (with and without `simd128`) and aarch64-linux-android (qemu). The bench checksums were
  recorded on the pre-optimization code.

## 4. Build size

`orr_web` (the browser client). Before this step: arena only, `release` profile.
After: arena **and** `PhysGame` (the physics code adds about 330 KB), per profile and `wasm-opt`.

| Build | wasm-bindgen output | + `wasm-opt -O3` | + `wasm-opt -Oz` | gzip of the -O3 file |
|---|---|---|---|---|
| before: arena only, `release` | 433 209 | not run | not run | 143 604 (of the unoptimized file) |
| after: arena + physics, `release` | 764 657 | 594 292 | 582 083 | 208 911 |
| after: arena + physics, `web` | 763 988 | 594 287 | 582 077 | 208 927 |
| after: arena + physics, `web-small` | 918 946 | 473 744 | 465 349 | 186 624 |

* `lto = "fat"`, `codegen-units = 1`, `panic = "abort"` and `debug = 0` change the size by under 1% (thin LTO and
  the wasm-bindgen output without debug info were already close). They are in the `web` profile because they
  do not hurt and make cross-crate inlining deterministic.
* `wasm-opt` is the lever: -22% (-O3) / -24% (-Oz). It is available from `apt install binaryen` or
  `npm i -g binaryen`; `tools/build_web.sh` runs it when found (`WASM_OPT_LEVEL=Oz` for the smallest file).
* `opt-level = "s"` plus `wasm-opt -Oz` is a further -22% against `web` + -O3 (-39% against the bare
  wasm-bindgen output), at a speed cost: the xxh3 frame checksum was 1.5x slower in one run, physics and fp
  operations were within noise (single runs). Use `WEB_PROFILE=web-small` where
  download size matters more than checksum speed.
* The GPU view package (`orr_web_gpu`, wgpu with WebGPU and WebGL2) is **2.6 MB** (-O3; gzip 977 KB). It is a
  separate package loaded on demand, so the simulation package stays at 0.6 MB.

## 5. Browser frame times (headless Chromium 141, software rendering)

`browser_e2e`, `PhysGame` with 300 bodies (mixer), 2 players, 600 ticks, relay with 35 ms latency each way:

| View | page frame interval (mean / p95 / max) | draw call in the page (mean / p95) | worker sim time per call (mean / p95 / max) |
|---|---|---|---|
| 2D canvas | 16.67 / 16.7 / 16.8 ms | 0.69 / 1.1 ms | 0.18 / 0.7 / 1.8 ms |
| WebGL2 (`orr_render`, SwiftShader) | 25.1 / 33.4 / 233 ms | 0.78 / 1.2 ms | similar |
| WebGPU (`orr_render`, SwiftShader) | 17.8 / 33.3 / 66.6 ms | 0.34 / 0.6 ms | similar |

The page frame interval is capped at 60 Hz by the headless vsync. The software rasterizer shares the 4 cores
with the sim and the native client, so the WebGL2/WebGPU interval numbers include that contention and the
first-frame pipeline compile (the max); on a real GPU the draw is a few instanced draw calls (the 2D shader draws
circles, boxes and capsules from one instance buffer). The "worker sim time" counts every `tick` call, most of
which find no tick due, so the mean is far below the tick cost; the p95/max are the real tick (with rollbacks).

## 6. Mobile 3D preset

`Settings3D::LOW` / `physics3d --low`: no MSAA, 512 texel shadow map, 12-segment spheres and capsules
(192 triangles per sphere instead of 1536). On lavapipe (software Vulkan, vertex bound, 1000 bodies, 1280x720,
60 frames, virtual X11 window):

| Settings | render call per frame | fps |
|---|---|---|
| default (4x MSAA, 2048 shadow map, 32 segments) | 291 ms | 3.4 |
| `--low` | **55 ms** (5.3x) | 16.8 |
| `--msaa 1` alone | 184 ms | 5.3 |
| `--mesh-segments 12` alone | 89 ms | 10.7 |
| `--shadow-map 512` alone | 298 ms | 3.3 |

A software rasterizer is dominated by vertex work and MSAA resolve; a mobile GPU is limited by fill rate and
bandwidth, so the shadow map size and MSAA will matter more there than lavapipe shows. No device was
available; measure on hardware before trusting the ratios.
