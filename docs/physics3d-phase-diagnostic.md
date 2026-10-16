# Native physics3d phase diagnostic

Evidence date: **2026-10-06 UTC**. Bounded diagnostic follow-up for
[issue #8](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/8).
This is separate from the [uninstrumented baseline and rejected ABBA experiment](physics3d-rollback-baseline.md).

## Outcome

**Awake work is concentrated in the Solve and Narrow markers; fully sleeping
work is concentrated in Gather. No runtime optimization is introduced or accepted.**

- Awake cases: Solve is **47.70–56.96%**, Narrow **23.93–25.19%**, Broad
  **5.07–14.66%**, Prepare **5.87–7.39%** of summed instrumented step time
- Sleeping fields: every measured tick has **zero awake bodies**, zero pairs,
  manifolds and solved points; only Gather/Finish fire. Gather is **96.13–96.26%**
- The sleeping rollback case spends **8.818%** of its derived step-plus-copy
  sum copying the frame; the awake rollback case spends **0.189%**
- All **1,520** instrumented ticks matched their ordinary-step replay's exact
  frame checksum and StepStats. All reset repetitions also matched exactly

These results support investigating the **awake substep loop and narrow phase**
and, separately, **sleeping gather/wake checks**. They do not identify which
internal operation dominates either marker, nor establish a safe code change.
Frame copy is a secondary candidate for sleeping resimulation, with only 20
observed copies here. No concrete new optimization hypothesis is validated;
allocation, cache, or instruction-level evidence would be needed to select one.
The rejected zero-impulse guard remains absent. There is no performance-gain,
portable budget, cross-platform, or general correctness claim from these timings.

## Fixed workload and boundaries

The new [example](../crates/orr_physics3d/examples/phase_costs.rs) imports the
unchanged [shared fixtures](../crates/orr_physics3d/tests/common/scenes.rs) used by
the unchanged [Criterion benchmark](../crates/orr_physics3d/benches/physics3d.rs).
It has no new dependency or manifest change. All scenes retain their original
physics configuration: **8 substeps × 1 velocity iteration**, sleeping disabled
for piles and the default sleep settings for fields.

| Cases | Warmup | Instrumented workload | Timed Frame copies |
| --- | --- | --- | --- |
| mixed_settling_500 / 1000 | 90 ordinary ticks | 200 ticks each; reset at indices 0 and 100 | 2 each |
| mixed_settled_500 / 1000 | 600 ordinary ticks | 200 ticks each; reset at indices 0 and 100 | 2 each |
| field_sleeping_500 / 1000 | 900 ordinary ticks | 200 ticks each; reset at indices 0 and 100 | 2 each |
| rollback8_mixed_settling_1000 | 90 ordinary ticks | 20 repetitions × 8 ticks; reset each repetition | 20 |
| rollback8_field_sleeping_1000 | 900 ordinary ticks | 20 repetitions × 8 ticks; reset each repetition | 20 |

Warmup Scratch is discarded; measurement begins with empty Scratch, which persists
across resets, exactly as in the benchmark. The first measured tick therefore
includes any first-use Scratch allocation work. This is an unprimed *Scratch*,
not a cold build/profile. No allocation counting or separate cold-allocation
experiment was added. Vectors holding diagnostic records are preallocated.

Every ordinary validation replay starts from the same warmed snapshot, with fresh
Scratch and the same reset schedule, and compares **each tick**, not just the last.
Validation, checksums, StepStats, assertions, tick-number increments, and output
are outside the timed step. Output is deferred until the case's measurements and
replay are done. The per-tick checks/checksums can still alter cache state before
the next tick. This differs from Criterion's uninterrupted workload.

`total_ns` brackets `step_probed`, **excluding `frame.set_tick`**; Criterion's
`tick` includes that increment. Copy measurements bracket the actual
`Frame::copy_from` resets, not clone, snapshot construction, or a separate copy
microbenchmark. The copy counts in single-step cases are only two, so their means
and ranges are weak diagnostics, not statistically established copy costs.

### What a phase marker actually covers

Markers report **phase end**, so elapsed time is attributed from the preceding
marker. Repeated occurrences are accumulated rather than overwritten.

| Marker | Actual interval at the measured source |
| --- | --- |
| Gather | Physics config/dense-slice lookup, body gather/order and orphan wake checks |
| Transforms | Transforms and bounding boxes, after the mover early-return decision |
| Broad | Sweep-flag update and candidate search; a repeated Broad interval also includes the preceding wake decision |
| Narrow | Contact generation and warm-start cache lookup |
| Integrate | Final wake decision/loop exit plus velocity workspace setup/damping |
| Prepare | `solver::prepare`: constraint/contact masses, friction basis, restitution and cached impulses |
| Solve | Entire `run_substeps`, including gravity, target updates, warm start, sequential solve, workspace position accumulation and friction storage, then velocity clamps |
| Sleep | Sleep timers and island update |
| Finish | Sleep counting, new cache construction, body position/rotation/write-back and cache store; on the no-mover path, only scratch cleanup/counting |

In particular, **Solve is not an isolated `solver::solve` measurement**, and warm
starting executes inside Solve, despite the Prepare enum's broad documentation.
Sleeping Gather also includes reading existing contact cache for orphan wake
checks. No repeated Broad/Narrow events occurred in this fixed sample; synthetic
self-checks cover accumulation and the legal early-return marker shape.

Callback clock reads, matching, duration accumulation, and function-layout effects
are included; bookkeeping after one timestamp is charged to the next interval.
The final callback/return remainder is reported as `tail_ns`. For every raw tick,
`sum(phase_ns) + tail_ns == total_ns`. A 1,000-empty-bracket timer calibration was
**20 ns minimum / 20 ns median / 22.77 ns mean / 40 ns maximum**. This neither
models the complete callback overhead nor measures its total effect. Nothing is
subtracted or corrected using this calibration.

## Observed instrumented costs

Arithmetic mean over the fixed tick sample; percentages are ratios of aggregate
phase nanoseconds to aggregate `total_ns`, not means of per-tick percentages.
All durations in the first table are **microseconds per timed tick**. “Other” is
Transforms + Integrate + Sleep + final tail; all nine components are retained
individually in [summary.json](measurements/physics3d-phases-2026-10-06/summary.json).
Zero entries denote absent marker calls, not rounded-away work.

| Case | Step µs | Broad % | Narrow % | Prepare % | Solve % | Finish % | Gather % | Other % |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mixed_settling_500 | 1155.741 | 7.87 | 25.00 | 6.56 | 53.09 | 4.00 | 0.30 | 3.18 |
| mixed_settling_1000 | 2675.061 | 12.06 | 24.61 | 6.41 | 50.26 | 3.59 | 0.25 | 2.83 |
| mixed_settled_500 | 1243.395 | 5.07 | 25.19 | 7.39 | 56.96 | 1.92 | 0.34 | 3.13 |
| mixed_settled_1000 | 2913.080 | 10.76 | 24.19 | 6.96 | 53.72 | 1.55 | 0.27 | 2.55 |
| field_sleeping_500 | 6.672 | 0.00 | 0.00 | 0.00 | 0.00 | 3.30 | 96.26 | 0.44 |
| field_sleeping_1000 | 11.025 | 0.00 | 0.00 | 0.00 | 0.00 | 3.60 | 96.13 | 0.27 |
| rollback8_mixed_settling_1000 | 2388.115 | 14.66 | 23.93 | 5.87 | 47.70 | 4.13 | 0.28 | 3.44 |
| rollback8_field_sleeping_1000 | 11.263 | 0.00 | 0.00 | 0.00 | 0.00 | 3.52 | 96.22 | 0.26 |

### Frame copy and derived operation sums

“Operation” is one tick for the six single-step cases and eight ticks for rollback.
Derived operation mean = `(sum(timed steps) + sum(timed copies)) / operations`.
This **is not contiguous end-to-end latency**: it omits all intervals between
separately timed components, including tick increments, checksums and stats.
It must not be compared as a candidate speedup against the prior Criterion data.

| Case | Copies | Mean µs/copy | Min–max µs/copy | Amortized ns/tick | Derived µs/operation | Copy % of derived sum |
| --- | --- | --- | --- | --- | --- | --- |
| mixed_settling_500 | 2 | 10.110 | 3.515–16.705 | 101.100 | 1155.843 | 0.009 |
| mixed_settling_1000 | 2 | 20.145 | 7.261–33.029 | 201.450 | 2675.263 | 0.008 |
| mixed_settled_500 | 2 | 12.849 | 4.026–21.672 | 128.490 | 1243.524 | 0.010 |
| mixed_settled_1000 | 2 | 22.639 | 8.934–36.343 | 226.385 | 2913.306 | 0.008 |
| field_sleeping_500 | 2 | 7.170 | 4.026–10.315 | 71.705 | 6.744 | 1.063 |
| field_sleeping_1000 | 2 | 11.211 | 8.863–13.560 | 112.115 | 11.137 | 1.007 |
| rollback8_mixed_settling_1000 | 20 | 36.106 | 7.771–98.696 | 4513.244 | 19141.028 | 0.189 |
| rollback8_field_sleeping_1000 | 20 | 8.713 | 8.132–11.016 | 1089.181 | 98.815 | 8.818 |

### StepStats and state evidence

Ranges are over measured ticks. Bodies includes the static floor/walls; the
nominal 500/1000 case size counts only dynamic bodies. Fully sleeping fields
retain sleeping contact caches even though StepStats reports zero solved contacts.

| Case | Bodies | Awake | Asleep | Pairs | Manifolds | Points |
| --- | --- | --- | --- | --- | --- | --- |
| mixed_settling_500 | 505 | 500 | 0 | 1380–1590 | 630–892 | 1007–1462 |
| mixed_settling_1000 | 1005 | 1000 | 0 | 3229–3679 | 1414–1989 | 2109–3028 |
| mixed_settled_500 | 505 | 500 | 0 | 1590–1594 | 968–973 | 1592–1600 |
| mixed_settled_1000 | 1005 | 1000 | 0 | 3700–3703 | 2135–2141 | 3242–3251 |
| field_sleeping_500 | 501 | 0 | 500 | 0 | 0 | 0 |
| field_sleeping_1000 | 1001 | 0 | 1000 | 0 | 0 | 0 |
| rollback8_mixed_settling_1000 | 1005 | 1000 | 0 | 3229–3319 | 1414–1503 | 2109–2201 |
| rollback8_field_sleeping_1000 | 1001 | 0 | 1000 | 0 | 0 | 0 |


| Case | Warmed snapshot checksum | Last measured checksum |
| --- | --- | --- |
| mixed_settling_500 | `0xae105f01d3d070ef` | `0x8136efccf67ad2df` |
| mixed_settling_1000 | `0xe671cc418fec854e` | `0xafad3f0a2a23f8d0` |
| mixed_settled_500 | `0xe79531632ae2cb31` | `0x4b10a9e24d6a450a` |
| mixed_settled_1000 | `0x9ce439518ab2585c` | `0x0eb1c9fa3948ba41` |
| field_sleeping_500 | `0x5c0af0bf34723d61` | `0x7a98cc08ecd0d237` |
| field_sleeping_1000 | `0xd86cd6ed42b0c7c5` | `0x17ad62560f23ee7d` |
| rollback8_mixed_settling_1000 | `0xe671cc418fec854e` | `0x8cdcbb83f017dd91` |
| rollback8_field_sleeping_1000 | `0xd86cd6ed42b0c7c5` | `0x25732313e8f9c4e9` |


Each tick's checksum, StepStats, phase call counts, nine phase durations, total and
tail are archived in [diagnostic.jsonl](measurements/physics3d-phases-2026-10-06/diagnostic.jsonl).
Repeated checksums are expected because each sample block restores the same
snapshot; this is not 1,520 independent scene states or randomized repetitions.

## Source, executable and resources

- Measured clean harness commit: `7282ca7c09e7c6895fb5a49a4ceafb0e81f056c1`
- Its tree: `97cf3baed8db542aa371dd0ced476f3d33f5747f`
- Parent: `d9fb5e0bd758ebed05990be904b106a17bbd07e7` (baseline documentation)
- Unchanged physics/code base: `a9e5ccbcc86ecf282179aa0b908ab5efd6d097c0`
- Preserved executable SHA-256: `f5d950eaa69bae315ba946c318730990533e65ac235f3fdfda73fe2aa04cf3ea`
- Executable size: **3,906,496 bytes**; hash equal before/after the invocation
- Rust **1.97.1**, LLVM **22.1.6**, Cargo **1.97.1**; native `x86_64-unknown-linux-gnu`
- Existing warm TUI release target, locked/offline Cargo, **one job**; default
  opt-level 3, thin LTO, one codegen unit, debug level 1, no overrides
- AMD EPYC 9V74 reported by `/proc/cpuinfo`, Linux 6.18.44. Logical CPUs 0–8 allowed;
  diagnostic launched with `taskset -c 0`, both in-run affinity observations `[0]`
- Logical affinity is not dedicated physical-core isolation. CPU topology and
  cgroup quota/throttling files were unavailable, and visible processes cannot
  exclude host workloads. The separate build was not CPU-0 pinned
- Build: **17.53 s**, exit 0, under its 300 s ceiling
- One total diagnostic invocation including warmups, calibration and replays:
  **7.01 s**, exit 0, under its 120 s ceiling
- Minimum free space during guard-loop polling: build **1,324,335,104 bytes**,
  diagnostic **1,323,225,088 bytes**. The later diagnostic-end observation was
  **1,323,130,880 bytes**; all observations remained above **1,073,741,824 bytes** (1 GiB)
- No retries, deleted artifacts, debug/cold profile, baseline reruns or extra
  diagnostic invocations. The monitor sampled free space about every 250 ms

The exact executable remains preserved locally at
`/workspace/scratch/edd3c9d10b8b/su-physics-phase-logs/phase_costs`; it is identified
but not bundled in the repository. Build/run logs, resource observations, workload process counts, tool versions,
source-input hashes, source patch, bounded runner and
summarizer are archived under
[measurements/physics3d-phases-2026-10-06](measurements/physics3d-phases-2026-10-06/metadata.json).
[SHA256SUMS](measurements/physics3d-phases-2026-10-06/SHA256SUMS) covers every archived file.
The repository observations retain only relevant build/diagnostic process-name
counts: all process command arguments, identities and unrelated rows are omitted.
Exact build/run commands remain in their result records. The archived CPU-info
text has only its trailing blank line normalized. Untouched originals remain in
the local evidence directory; their hashes, archived hashes and precise
transformations are recorded in
[archive-transformations.json](measurements/physics3d-phases-2026-10-06/archive-transformations.json).
Raw timing JSONL, numerical summaries, source hashes and executable identity are
unchanged by these archive-only transformations.
The helper is the only code change; core, solver, iterations, fixtures, goldens,
baseline benchmark, manifests and dependency lockfile are unchanged.

## Reproduction and verification scope

Use an already warm release target, sufficient disk headroom and the same toolchain.
The archived [bounded runner](measurements/physics3d-phases-2026-10-06/run_diagnostic.py)
records the exact local paths and guards used for this run. Its no-repeat guard
requires a fresh output directory; adapt only paths for another environment.
The equivalent build/executable commands are:

```sh
cargo +1.97.1 build --release -p orr_physics3d --example phase_costs --locked --offline -j1
taskset -c 0 "$CARGO_TARGET_DIR/release/examples/phase_costs" > diagnostic.jsonl
```

Those bare commands do not enforce resource bounds; use a bounded wrapper when
reproducing. Do not run the second command as another observation of this archived
experiment. The pure-stdlib summarizer can recalculate the checked-in evidence:

```sh
python docs/measurements/physics3d-phases-2026-10-06/summarize.py docs/measurements/physics3d-phases-2026-10-06
(cd docs/measurements/physics3d-phases-2026-10-06 && sha256sum -c SHA256SUMS)
```

Verified: release build, formatting of the new example, `git diff --check`,
synthetic accumulator assertions, all measured per-tick checksum/StepStats parity,
reset replay equality, exact eight workload counts, phase/copy accounting and raw
artifact hashes. Independent read-only review passed the pre-run source, final
numeric/checksum tables, phase semantics, relative links, archive hashes and
archive transformation provenance. Full workspace tests, golden suite, Clippy,
Criterion reruns, allocation profiling and non-native targets were **not run** in
this bounded diagnostic task. Existing goldens were not edited. Timing results
alone do not validate correctness; the parity assertions apply only to these
specific source/build/workloads.
