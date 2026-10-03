# Windows release baseline — 2026-10-03

Final, separately captured release-mode baselines for the same clean source revision. Each mode has three fresh-process repetitions; statistics below use median-of-run medians / median-of-run p95s / maximum of run maxima, in milliseconds. Full per-run evidence is in [summary.json](summary.json), and raw captures are linked as [raw-evidence.zip](raw-evidence.zip) with its [raw index](raw-index.json).

## Capture identity

- Source: `120efa6efa4b7ba6c82a88be7cb402e1e69ca2a1`; before and after each campaign: clean tree, 56 source files, SHA-256 `0db1093287132450c36c9990f7971caf8def8dee30e7336d8d0b9b063b7d8887` (both modes match).
- Default Cargo features, locked release builds; Rust `1.97.1` (`x86_64-pc-windows-msvc`), Cargo `1.97.1`; Windows 11 build `10.0.26200`, Intel Core i7-14700, 28 logical CPUs, Balanced power scheme.
- The editor test uses `egui_kittest`: `demo_editor` has 49 fixture bodies; the UI workload uses `demo_editor_1000_bodies`, 1,049 actual bodies. Renderer workloads use 1,000 instances at 640×360.
- Each campaign compiled the existing editor-measure and renderer test targets with `--release --locked --no-run`, then ran each of its four measurement commands serially three times. All 12 process rows per mode exited 0, were not timed out/interrupted, and passed record validation.

## Editor measurements

Values are ms. Drag first is the first 50-frame interaction group; steady is the following 39 measured drag rows. UI phases contain 60 edit frames and 240 frames each for 1× and 4× play per process. Hardware/software labels identify the surrounding renderer campaign; the editor harness itself has no GPU viewport.

| Campaign | Measure | Samples per process | Median / p95 / max |
|---|---|---:|---:|
| Hardware | First drag, 50 frames | 1 group × 3 processes | 8.652 / 9.079 / 9.079 |
| Hardware | Steady drag, 39 rows | 39 × 3 | 0.611 / 3.784 / 4.515 |
| Hardware | UI edit | 60 × 3 | 1.150 / 1.396 / 1.650 |
| Hardware | UI play 1x | 240 × 3 | 1.248 / 1.614 / 2.251 |
| Hardware | UI play 4x | 240 × 3 | 1.242 / 1.725 / 2.181 |
| Software | First drag, 50 frames | 1 group × 3 processes | 8.428 / 8.650 / 8.650 |
| Software | Steady drag, 39 rows | 39 × 3 | 0.656 / 3.751 / 4.371 |
| Software | UI edit | 60 × 3 | 1.177 / 1.457 / 2.227 |
| Software | UI play 1x | 240 × 3 | 1.222 / 1.612 / 2.279 |
| Software | UI play 4x | 240 × 3 | 1.244 / 1.677 / 2.142 |

First-drag p95 with three observations is the nearest-rank maximum. Per-process summaries and all exact observations remain available in `summary.json` and the raw records.

## Renderer measurements

Frame wall time is CPU render-call return/backpressure, not GPU completion or FPS. `cold` is the first frame after renderer construction; renderer initialization and target creation are excluded. Ten warmup frames are retained separately from the 30 steady frames. Each table cell is median-of-run median / median-of-run p95 / maximum of run maxima (ms).

| Mode | Renderer | Frame class | wall ms | prepare ms | encode ms | submit ms | post-batch readback ms per process (shown once) |
|---|---|---|---:|---:|---:|---:|---|
| Hardware | 2D default | cold | 0.337 / 0.337 / 0.369 | 0.025 / 0.025 / 0.026 | 0.015 / 0.015 / 0.034 | 0.296 / 0.296 / 0.310 | 3.728,3.750,3.967 |
| Hardware | 2D default | warmup | 0.044 / 13.922 / 16.766 | 0.010 / 13.451 / 16.399 | 0.002 / 0.010 / 0.012 | 0.032 / 0.461 / 0.476 | — |
| Hardware | 2D default | steady | 0.033 / 0.314 / 0.688 | 0.008 / 0.145 / 0.174 | 0.001 / 0.005 / 0.012 | 0.024 / 0.204 / 0.628 | — |
| Hardware | 3D low | cold | 0.610 / 0.610 / 0.695 | 0.248 / 0.248 / 0.257 | 0.021 / 0.021 / 0.022 | 0.356 / 0.356 / 0.415 | 3.953,4.861,4.009 |
| Hardware | 3D low | warmup | 0.207 / 0.387 / 0.511 | 0.105 / 0.186 / 0.186 | 0.004 / 0.010 / 0.010 | 0.111 / 0.370 / 0.401 | — |
| Hardware | 3D low | steady | 0.048 / 0.462 / 0.582 | 0.013 / 0.130 / 0.171 | 0.002 / 0.007 / 0.012 | 0.032 / 0.379 / 0.533 | — |
| Hardware | 3D default | cold | 0.723 / 0.723 / 0.835 | 0.307 / 0.307 / 0.424 | 0.022 / 0.022 / 0.022 | 0.387 / 0.387 / 0.391 | 9.248,8.999,6.421 |
| Hardware | 3D default | warmup | 0.103 / 0.456 / 0.465 | 0.014 / 0.132 / 0.141 | 0.003 / 0.006 / 0.009 | 0.042 / 0.348 / 0.376 | — |
| Hardware | 3D default | steady | 0.173 / 0.448 / 0.571 | 0.077 / 0.158 / 0.206 | 0.003 / 0.009 / 0.013 | 0.119 / 0.415 / 0.493 | — |
| Software | 2D default | cold | 0.919 / 0.919 / 1.004 | 0.038 / 0.038 / 0.045 | 0.021 / 0.021 / 0.022 | 0.860 / 0.860 / 0.936 | 154.168,158.748,150.408 |
| Software | 2D default | warmup | 0.133 / 0.210 / 0.240 | 0.037 / 0.076 / 0.113 | 0.006 / 0.011 / 0.018 | 0.086 / 0.118 / 0.134 | — |
| Software | 2D default | steady | 0.121 / 0.188 / 0.224 | 0.035 / 0.054 / 0.087 | 0.004 / 0.011 / 0.018 | 0.085 / 0.127 / 0.158 | — |
| Software | 3D low | cold | 1.932 / 1.932 / 2.137 | 0.049 / 0.049 / 0.063 | 0.025 / 0.025 / 0.026 | 1.855 / 1.855 / 2.045 | 1157.565,1246.057,1255.509 |
| Software | 3D low | warmup | 0.467 / 0.621 / 0.660 | 0.063 / 0.107 / 0.119 | 0.011 / 0.019 / 0.026 | 0.401 / 0.516 / 0.523 | — |
| Software | 3D low | steady | 0.480 / 0.800 / 0.989 | 0.069 / 0.095 / 0.141 | 0.011 / 0.040 / 0.228 | 0.406 / 0.664 / 0.901 | — |
| Software | 3D default | cold | 4.537 / 4.537 / 4.713 | 0.056 / 0.056 / 0.058 | 0.026 / 0.026 / 0.026 | 4.451 / 4.451 / 4.628 | 8690.276,8843.291,8661.702 |
| Software | 3D default | warmup | 2.434 / 2.875 / 2.877 | 0.094 / 0.107 / 0.126 | 0.015 / 0.020 / 0.022 | 2.342 / 2.774 / 2.788 | — |
| Software | 3D default | steady | 2.838 / 3.666 / 3.767 | 0.107 / 0.166 / 0.193 | 0.019 / 0.032 / 0.041 | 2.699 / 3.515 / 3.608 | — |

Readback excludes the frame samples (`timing_included_in_frame_samples=false`) and uses a 921,600-byte image. The three readback durations are shown per process so slow software-readback tails stay visible. CPU prepare/encode/submit timings are sequential, non-overlapping stages (renderer.rs:181/201/204 and renderer3d.rs:452/528/531). `wall_ms` includes additional unmeasured costs; do not add separate per-stage medians or p95s as a wall-time estimate.

Adapter identity (same across repetitions):

- **Hardware mode:** NVIDIA GeForce RTX 5070, Vulkan, discrete GPU, NVIDIA driver 596.49; `software=false`, driver metadata available.
- **Software mode:** Microsoft Basic Render Driver, Direct3D 12, CPU adapter, driver `10.0.26100.9278`; `software=true`. The optional `driver_info` string is empty and recorded as unavailable, not treated as a failed final validation.

## Editor diagnostic samples

These are internal recent-window diagnostics, not end-to-end editor latency. Each entry gives p95 ms and `samples/total_samples` for repetitions 1/2/3. Editor diagnostic categories may overlap and are not additive. The six-hundred-tick editor step is synchronous; `sync_erp_wait` captures its blocking ERP wait. `async_request` below is a later post-to-ingest recent window collected after the step, not the step latency.

| Mode / window | Async request | UI frame | Pump | Sync ERP wait | Snapshot extract |
|---|---:|---:|---:|---:|---:|
| Hardware / drag | 0.830 (120/479)<br>0.807 (120/467)<br>0.910 (120/511) | 0.692 (120/171)<br>0.639 (120/168)<br>0.676 (120/179) | 0.176 (120/183)<br>0.139 (120/180)<br>0.165 (120/191) | 0.348 (14/14)<br>0.368 (14/14)<br>0.332 (14/14) | 0.011 (19/19)<br>0.011 (19/19)<br>0.013 (19/19) |
| Hardware / edit | 9.685 (29/29)<br>9.504 (29/29)<br>11.024 (29/29) | 1.361 (67/67)<br>1.340 (67/67)<br>1.509 (67/67) | 0.010 (77/77)<br>0.010 (77/77)<br>0.009 (77/77) | 2.904 (120/1012)<br>2.913 (120/1012)<br>2.746 (120/1012) | 0.085 (3/3)<br>0.066 (3/3)<br>0.071 (3/3) |
| Hardware / play 1x | 9.672 (47/47)<br>9.487 (47/47)<br>11.009 (47/47) | 1.537 (120/312)<br>1.468 (120/312)<br>1.481 (120/312) | 0.071 (120/322)<br>0.065 (120/322)<br>0.062 (120/322) | 2.904 (120/1015)<br>2.913 (120/1015)<br>2.746 (120/1015) | 0.095 (24/24)<br>0.071 (25/25)<br>0.110 (25/25) |
| Hardware / play 4x | 9.146 (60/60)<br>9.107 (60/60)<br>10.424 (62/62) | 1.783 (120/552)<br>1.523 (120/552)<br>1.545 (120/552) | 0.068 (120/562)<br>0.064 (120/562)<br>0.065 (120/562) | 2.904 (120/1017)<br>2.913 (120/1017)<br>2.746 (120/1017) | 0.098 (50/50)<br>0.088 (52/52)<br>0.085 (52/52) |
| Software / drag | 0.968 (120/459)<br>0.903 (120/455)<br>1.002 (120/467) | 0.743 (120/166)<br>0.722 (120/165)<br>0.711 (120/168) | 0.191 (120/178)<br>0.188 (120/177)<br>0.193 (120/180) | 0.399 (14/14)<br>0.343 (14/14)<br>0.396 (14/14) | 0.012 (19/19)<br>0.013 (19/19)<br>0.010 (19/19) |
| Software / edit | 9.316 (29/29)<br>9.685 (29/29)<br>9.794 (29/29) | 1.404 (67/67)<br>1.496 (67/67)<br>1.480 (67/67) | 0.011 (77/77)<br>0.010 (77/77)<br>0.011 (77/77) | 2.786 (120/1012)<br>2.990 (120/1012)<br>2.787 (120/1012) | 0.098 (3/3)<br>0.074 (3/3)<br>0.056 (3/3) |
| Software / play 1x | 9.303 (47/47)<br>9.673 (45/45)<br>9.781 (47/47) | 1.464 (120/312)<br>1.484 (120/312)<br>1.502 (120/312) | 0.067 (120/322)<br>0.063 (120/322)<br>0.064 (120/322) | 2.786 (120/1015)<br>2.990 (120/1015)<br>2.787 (120/1015) | 0.102 (25/25)<br>0.096 (24/24)<br>0.083 (25/25) |
| Software / play 4x | 8.875 (60/60)<br>9.559 (60/60)<br>9.181 (60/60) | 1.527 (120/552)<br>1.615 (120/552)<br>1.604 (120/552) | 0.068 (120/562)<br>0.064 (120/562)<br>0.063 (120/562) | 2.786 (120/1017)<br>2.990 (120/1017)<br>2.787 (120/1017) | 0.098 (51/51)<br>0.096 (50/50)<br>0.084 (50/50) |


### Six-hundred-tick synchronous step

The editor harness reports host simulation at 0.001 ms precision; this is the harness's host-simulation measurement, not a pure engine-kernel timing. The same run's `sync_erp_wait` p95/last/max and sample counts are shown alongside the separate post-step async window.

| Mode | Rep | Harness host simulation (ms/tick) | Blocking `sync_erp_wait` p95 / last / max ms (samples/total) | Later `async_request` p95 / last / max ms (samples/total) |
|---|---:|---:|---:|---:|
| Hardware | 1 | 0.499 | 2.922 / 299.169 / 299.169 (120/1020) | 9.146 / 1.457 / 9.694 (60/60) |
| Hardware | 2 | 0.494 | 2.914 / 295.982 / 295.982 (120/1020) | 9.107 / 2.387 / 9.511 (62/62) |
| Hardware | 3 | 0.500 | 2.750 / 300.000 / 300.000 (120/1020) | 10.424 / 1.794 / 11.032 (62/62) |
| Software | 1 | 0.502 | 2.816 / 300.913 / 300.913 (120/1020) | 8.875 / 3.272 / 9.322 (60/60) |
| Software | 2 | 0.500 | 3.187 / 299.924 / 299.924 (120/1020) | 9.559 / 1.463 / 9.693 (60/60) |
| Software | 3 | 0.502 | 3.195 / 301.134 / 301.134 (120/1020) | 9.181 / 1.678 / 9.827 (60/60) |

## Reproduction and validation

Use the [fixed workload procedure](../../editor-render-baseline.md) and a clean
worktree at the measurement commit above. Reserve a quiet local test lane, build
first, and use fresh output directories. The two serial collector commands were:

```sh
python tools/editor_release_baseline.py --output target/editor-release-baseline/hardware-final-20261003T0708Z --repetitions 3 --gpu-mode default
python tools/editor_release_baseline.py --output target/editor-release-baseline/software-final-20261003T0708Z --repetitions 3 --gpu-mode software
```

Exact commands, environment allowlists and start/end times are in each archived
manifest. The final hardware campaign ran from 07:08:42 to 07:09:03 UTC; software
followed and finished at 07:09:55 UTC. Other workers' compiler and test loads were
excluded from this window; normal desktop/app services remained. Dependency
artifacts were warm, so these prebuild durations are not cold compilation costs.

Collector regression tests passed **15/15**. All three changed release test
targets compiled, and focused strict Clippy passed for `measure`, `gpu` and
`gpu3d`. The archive retains both earlier strict-Clippy failures: benchmark-local
clock annotations were corrected after the first; the second is the unchanged
editor library-test `duplicate_mod` warning. No shared lint configuration or
mandatory CI gate was relaxed. These local results do not replace exact-head CI.

The raw ZIP is 602,007 bytes with 265 indexed entries and SHA-256
`5fbe098894d39cce96d8b92984e8946e86c20409c7c10db35f231265c2c3720c`.
Its CRC, entry sizes/hashes and captured source bytes were independently checked.

## Scope and historical captures

The editor rows are synthetic `egui_kittest` measurements; they do not represent native GUI startup or physical input. GPU rows do not measure GPU completion, FPS, or remote ERP/network behavior. Do not infer a speedup against debug builds, CI, previous revisions, or between hardware and software modes.

The earlier `456a6358a89c1ebe3cc79430d2d210025cabb75d` hardware capture remains complete. Its separately preserved software capture is incomplete because validation rejected the software adapter’s empty optional `driver_info`; this was a validation false positive. Those captures are retained as historical evidence and are not pooled or compared with this final source revision.

Raw bundle/index preserve every run’s raw stdout/stderr, argv, environment allowlist, timestamps, exit status, build binary identities, renderer JSONL records, and validation data.
