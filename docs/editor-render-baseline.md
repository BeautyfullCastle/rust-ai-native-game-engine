# Editor and renderer release baseline

This procedure measures the existing editor diagnostics and renderer FrameStats
under fixed workloads. It preserves failed runs and separates first observations,
steady samples and GPU readback. It does not change the 40 ms drag assertion or
claim an engine optimization from measurements alone.

## Fixed workloads and limits

The editor workload uses egui_kittest at 1500 by 900 logical pixels, a local host
thread and the physics demo scene. It has no GPU viewport or network ERP
connection. Report its UI, pump, blocking ERP wait, async request and snapshot
samples as that local harness's observations. A metric with no samples is
unavailable, rather than a measured zero latency. Those durations may overlap and
must not be summed into an invented end-to-end duration.

The first drag is the first observed move after the existing fixed harness
settling sequence. Its other 39 moves form the steady sample group. This first
move is not OS process startup or the first frame of an interactive native
editor. The larger scene adds 1000 bodies to the demo; record the actual body
count. Collect 60 edit frames, 240 play frames at 1x and 240 at 4x separately.

The headless GPU workload renders a fixed grid of 1000 instances into a 640 by
360 texture. Keep 2D default, 3D LOW and 3D default as separate cases. Record the
actual adapter, software flag, graphics API, driver and renderer settings.
Each case has one first frame after renderer/target construction, ten warmup
frames and thirty steady frames. Renderer construction and shader setup before
that first call are outside its frame timing.

FrameStats prepare/encode/submit and the render-call wall interval are CPU
observations. API waits and backpressure can affect them. They are not GPU
execution/completion times, display latency or interactive FPS. Time the
post-batch texture readback separately and exclude it from frame distributions;
it may wait for queued GPU work. Only the app's offscreen target is read back.

## Source, environment and scheduling

Use a clean independent worktree from the latest published
`su/engine-correctness-hardening` and record its exact SHA, tree and source/scene
hashes. Freeze release profile, default features, commands, workload sizes,
repetition count, environment and adapter selection before comparing results.
Record OS, CPU, graphics inventory, driver, power mode and toolchain. Unavailable
metadata remains unavailable.

Run at least three independent test processes per case. Each process starts
a new harness/device; do not combine its first sample with another process's
steady samples. Preserve every scheduled repetition, exit code and raw log.
A 40 ms failure belongs in the result and must not be removed by selecting a
later successful run.

Reserve the local Cargo lane through issue #28. Build first, then measure in a
separate quiet window after the other worker's Rust builds and checks finish.
Record actual start/end times and any unrelated load. Do not present mixed-load
samples as a controlled baseline or a before/after improvement. Hardware and
software results need separate output directories and summaries.

## Commands

The following explicit tests reuse the existing assertions. Build/test commands
must remain serial on the shared Windows host.

```sh
cargo test --locked -p orr_editor --release --test measure -- --nocapture --test-threads=1
cargo test --locked -p orr_render --release --test gpu release_baseline_2d -- --ignored --exact --nocapture --test-threads=1
cargo test --locked -p orr_render --release --test gpu3d release_baseline_3d_low -- --ignored --exact --nocapture --test-threads=1
cargo test --locked -p orr_render --release --test gpu3d release_baseline_3d_default -- --ignored --exact --nocapture --test-threads=1
```

GPU runs require `ORR_REQUIRE_GPU=1`. Set `ORR_BASELINE_GPU_MODE=hardware` to
request the ordinary hardware-preferred adapter path, or `software` for the
forced-software path. Hardware preference alone does not prove hardware was
selected: inspect the actual adapter/software metadata. A software fallback
cannot establish a hardware baseline.

PowerShell examples:

```powershell
$env:ORR_REQUIRE_GPU = '1'
$env:ORR_BASELINE_GPU_MODE = 'hardware'
cargo test --locked -p orr_render --release --test gpu release_baseline_2d -- --ignored --exact --nocapture --test-threads=1
```

These GPU benchmark tests are opt-in/ignored during ordinary workspace tests.
That avoids turning variable frame costs into a replacement mandatory gate.
Existing GPU pixel/stat assertions and the whole determinism workflow remain
required and unchanged.

## Raw records and comparison

Machine records begin with `ORR_BASELINE ` followed by JSON. Retain the full
stdout/stderr alongside parsed records so a parser failure cannot erase the
observation. Editor records identify the scene, actual body count, logical size,
harness backend, frame phase and metric sample counts. GPU records identify the
scene, target size, settings, actual adapter/backend/driver, frame class and
existing CPU/render counters.

The collector freezes metadata and schedules all repetitions before execution.
It checks record counts and numeric types, preserves ordinary test failures,
and refuses a complete baseline when records are missing, skipped, malformed or
conditions differ across repetitions. Compare medians/p95/max only for matching
scene/profile/backend/adapter/settings and sample groups. Each successful prebuild
records the actual Cargo test executable path, size and SHA-256 separately from
the clean source/tree identity and measured timings. Aborted plans retain the IDs
of planned but unstarted commands.

The collector interface is:

```sh
python tools/editor_release_baseline.py --self-test
python tools/editor_release_baseline.py --output target/editor-release-baseline/hardware-001 --repetitions 3 --gpu-mode default
python tools/editor_release_baseline.py --output target/editor-release-baseline/software-001 --repetitions 3 --gpu-mode software
```

`default` requests the hardware-preferred path; actual metadata establishes the
selected device. The collector first builds all three release test targets with
locked dependencies, then records each process's frame samples. Use a new ignored
output directory every time; existing evidence is never overwritten. Preserve
the manifest, raw logs, per-command outcomes and complete parsed records.

No measurement results have been collected for this issue yet. The collector
and Rust workloads still require their focused validation before a baseline can
be published.
