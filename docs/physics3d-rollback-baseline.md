# Native physics3d / rollback baseline and rejected zero-impulse experiment

Evidence date: **2026-10-06 UTC**. This is a documentation-only record for
[issue #8](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/8).

## Outcome

**No runtime optimization was retained; #8 remains open.** Two native baseline
runs completed, followed by one separate four-run ABBA evaluation of a
zero-impulse guard. The candidate showed no consistent overall benefit and
material regression risk in settled scenes, so both its guard and candidate-only
tests were reverted. Production source and golden checksums are unchanged.

- Baseline repeat experiment: 2 runs × 8 cases × 20 aggregate samples = **320 samples**
- Candidate experiment: A1/B1/B2/A2 × 8 cases × 20 aggregate samples = **640 samples**
- These are distinct experiments, not six candidate repetitions or 960 independent
  single-step latency observations. They establish no accepted performance gain
- This evidence does not establish a portable 4 ms target, full Session/network
  rollback latency, a phase breakdown, or cross-platform correctness

## Source, binaries and environment

The measured clean source commit was
`a9e5ccbcc86ecf282179aa0b908ab5efd6d097c0`, tree
`05ef672c3e515efd0e96ff81a44c1dfdd748c7a8`. The published equivalent is
[PR #87](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/87),
commit [b5ea974a2bda490efb336346fd6208a726a1e1cd](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/commit/b5ea974a2bda490efb336346fd6208a726a1e1cd);
its tree was verified equal to the measured tree. The local commit identifier is
provenance, not a claim that it exists on the remote.

| Artifact | SHA-256 | Bytes |
|---|---|---:|
| Preserved baseline executable | `13089aace80a803e35d58aefcf874103097ab722eca17e1c2fdabd902404d70a` | 15,506,408 |
| Preserved rejected-candidate executable | `3c2980cf1ad24178ff902202f16f8e10c2c76d266d44b08a62a1316b9f58d891` | 15,506,560 |
| Archived candidate patch | `222307d596123779b38342309098f9a1021c1b256f52984a8d1adad6d7b1e3d7` | 4,233 |

The baseline binary was hash-verified before/after its two runs, then copied and
verified before the candidate rebuild reused the warm target. ABBA ran the
preserved baseline/candidate copies. The shared release target subsequently
contained the candidate build: its filename alone cannot identify the baseline.
Code and binary-layout effects cannot be separated causally in these timings.
Binaries are identified here but are not bundled in this repository archive.

- Rust **1.97.1**, rustc commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452`, LLVM **22.1.6**
- Cargo **1.97.1**, commit `c980f4866141969fab6254a680546a277789d6f0`
- Criterion **0.5.1**; native target `x86_64-unknown-linux-gnu`; Debian 13 / Linux 6.18.44
- Existing `bench` profile inherits `release`: default opt-level **3**, thin LTO,
  **1** codegen unit, debug **1**. No profile or Rust-flag overrides
- Existing warm build cache; locked/offline Cargo; one build job
- CPU model reported by `/proc/cpuinfo`: **AMD EPYC 9V74 80-Core Processor**
- Allowed logical CPUs **0–8**; benchmark processes pinned with `taskset -c 0`
  (util-linux 2.41.5). Every postlaunch affinity observation was `[0]`. The initial
  pre-taskset observation inherited 0–8
- Logical affinity is **not dedicated physical-core isolation**. Usable CPU
  topology and cgroup quota/throttling data were unavailable. The visible process
  namespace cannot exclude other host workloads; observed load includes the benchmark

Exact compiler output, source-input hashes, run timestamps, resource results and
archived original-file hashes are in
[provenance.json](measurements/physics3d-2026-10-06/provenance.json).

## What was measured

The unchanged [benchmark](../crates/orr_physics3d/benches/physics3d.rs) uses the
[shared deterministic scenes](../crates/orr_physics3d/tests/common/scenes.rs).
The numeric suffix is the dynamic-body count; arena/floor statics are additional.

| Cases | Scene preparation outside measurement | One measured iteration |
|---|---|---|
| `mixed_settling_500`, `mixed_settling_1000` | Mixed sphere/box/capsule pile; sleeping disabled; 90 ticks | One physics step; snapshot restored once per 100 steps |
| `mixed_settled_500`, `mixed_settled_1000` | Same mixed pile; sleeping disabled; 600 ticks | One physics step; snapshot restored once per 100 steps |
| `field_sleeping_500`, `field_sleeping_1000` | Separated small groups; sleeping enabled; 900 ticks | One physics step; snapshot restored once per 100 steps |
| `rollback8_mixed_settling_1000` | Mixed pile; sleeping disabled; 90 ticks | Snapshot restore and eight physics steps |
| `rollback8_field_sleeping_1000` | Separated groups; sleeping enabled; 900 ticks | Snapshot restore and eight physics steps |

There are exactly **eight cases**. Scene preparation totals **4,170 ticks per
invocation**, outside Criterion measurement. Single-step timings span repeating
100-tick windows and include the amortized snapshot copy. `rollback8` is a
restore-plus-resimulation microbenchmark, not a full Session/network rollback
response. There is no settled rollback8 or 500-body rollback8 case, and no
broad-phase/narrow-phase/solver/integration/copy phase attribution.

## Commands and resource bounds

Both builds used the unchanged command:

```sh
cargo bench -p orr_physics3d --bench physics3d --no-run --locked --offline -j1
```

Each complete benchmark invocation used the following arguments. `<binary>`
identifies one of the exact hashed executables above; each `<unique-name>` and
initially absent `CRITERION_HOME` directory is recorded in `provenance.json`.

```sh
taskset -c 0 <binary> --bench --noplot --sample-size 20 \
  --warm-up-time 1 --measurement-time 2 --nresamples 1000 \
  --save-baseline <unique-name>
```

No previous Criterion baseline data was compared. The harness retained Criterion's
**Auto** sampling choice; actual Flat/Linear modes are archived per case/run.
Twenty aggregate samples, 1 s warm-up and 2 s measurement were requested, with
1,000 bootstrap resamples. Actual measured durations can exceed the request.

Each build had a **300 s** wall cap; each complete benchmark invocation had a
**180 s** cap. A **1 GiB (1,073,741,824-byte)** free-disk floor was checked every
**250 ms**; process/load/affinity snapshots were recorded every **5 s**. No benchmark-build
or benchmark guard fired. Baseline build elapsed **27.795 s**, minimum free disk
**1,391,218,688 bytes**; candidate build **30.814 s**, minimum **1,350,172,672 bytes**.
These are warm-target build observations, not clean-build performance comparisons.

| Experiment/run | Binary | Wall seconds | Minimum free bytes | Observed 1-min load min/max | Exit |
|---|---|---:|---:|---:|---:|
| baseline_repeat / pass1 | baseline | 33.299 | 1,404,170,240 | 0.26 / 0.55 | 0 |
| baseline_repeat / pass2 | baseline | 33.061 | 1,403,740,160 | 0.59 / 0.75 | 0 |
| zero_impulse_abba / a1 | baseline | 32.054 | 1,362,714,624 | 0.31 / 0.58 | 0 |
| zero_impulse_abba / b1 | candidate | 31.050 | 1,361,874,944 | 0.62 / 0.77 | 0 |
| zero_impulse_abba / b2 | candidate | 31.591 | 1,361,039,360 | 0.77 / 0.86 | 0 |
| zero_impulse_abba / a2 | baseline | 33.087 | 1,360,072,704 | 0.86 / 0.92 | 0 |

## Baseline repeat experiment: no source change

All cases used Linear sampling in both passes. Values below are Criterion slope
estimates in **milliseconds per benchmark iteration**, with **95% confidence
intervals**. These intervals are not worst-case or real-time latency bounds.
The change column is same-binary drift, not an optimization gain.

| Case | Pass 1 ms [95% CI] | Pass 2 ms [95% CI] | Pass 2 / pass 1 change | Actual iterations 1 / 2 | Measured seconds 1 / 2 |
|---|---:|---:|---:|---:|---:|
| mixed_settling_500 | 1.243443 [1.210334, 1.272870] | 1.188463 [1.178923, 1.199906] | -4.42% | 1,680 / 1,680 | 2.071 / 2.002 |
| mixed_settling_1000 | 2.768268 [2.707331, 2.863971] | 2.747375 [2.712131, 2.780556] | -0.75% | 840 / 840 | 2.326 / 2.306 |
| mixed_settled_500 | 1.344173 [1.308867, 1.393814] | 1.353850 [1.335319, 1.369397] | +0.72% | 1,680 / 1,470 | 2.262 / 1.985 |
| mixed_settled_1000 | 3.007355 [2.997189, 3.016933] | 3.040473 [2.988611, 3.103349] | +1.10% | 840 / 840 | 2.526 / 2.546 |
| field_sleeping_500 | 0.005509 [0.005312, 0.005769] | 0.005529 [0.005341, 0.005827] | +0.36% | 361,200 / 375,690 | 2.029 / 2.124 |
| field_sleeping_1000 | 0.010362 [0.010226, 0.010504] | 0.011002 [0.010505, 0.011459] | +6.18% | 191,520 / 187,530 | 1.980 / 2.025 |
| rollback8_mixed_settling_1000 | 18.783774 [18.688255, 18.877714] | 18.931877 [18.528469, 19.341813] | +0.79% | 210 / 210 | 3.969 / 3.950 |
| rollback8_field_sleeping_1000 | 0.090234 [0.089696, 0.090802] | 0.091666 [0.090115, 0.093837] | +1.59% | 21,840 / 21,210 | 1.973 / 1.951 |

Same-binary slope changes ranged from **−4.42% to +6.18%**. The 500-body settling
and 1000-body sleeping slope intervals did not overlap (the latter barely so).
Two brief runs cannot isolate a cause or establish a universal noise threshold.
Per-sample normalized-time CV ranged from **1.18% to 19.94%**. The largest was
pass-1 awake rollback, whose single-iteration first sample took **36.144 ms**
versus an **18.784 ms** slope estimate. No outliers were manually removed.

Both awake rollback passes warned that 20 Linear samples would not fit the
requested 2 s measurement: Criterion estimated **3.997 / 3.905 s** and measured
**3.969 / 3.950 s**. This was automatic duration extension, not an interruption or
manual parameter change. Historical results on other hardware/settings are not
controlled comparators for this baseline.

## Rejected candidate: one ABBA sequence

The proposed change to `solver::apply()` was only the four production lines
adding a comment and `if lam == FP::ZERO { return; }`. Constraint traversal,
iteration counts, nonzero arithmetic order, quality settings and goldens were
unchanged. The exact rejected diff is preserved as
[candidate.patch.txt](measurements/physics3d-2026-10-06/candidate.patch.txt):
**80 added lines**, comprising 4 production lines and 76 test/spacing lines.
This file is an inactive evidence artifact, not a patch to apply or a shipped
optimization. The guard and candidate-only tests were reverted together.

Order was **A1 (baseline), B1 (candidate), B2 (candidate), A2 (baseline)**.
Pair 1 compares **B1/A1**; pair 2 compares **B2/A2**. Negative percentages mean
faster. These two pair ratios are observations, not statistical proof of causation.

| Case | Slope pair 1 | Slope pair 2 | Median pair 1 | Median pair 2 |
|---|---:|---:|---:|---:|
| mixed_settling_500 | +0.82% | -5.61% | -0.06% | -3.94% |
| mixed_settling_1000 | -6.59% | +2.35% | -4.80% | +3.56% |
| mixed_settled_500 | -4.68% | +19.20% | +1.62% | +11.79% |
| mixed_settled_1000 | +3.37% | +14.74% | +1.74% | +7.14% |
| field_sleeping_500 | -5.81% | -0.60% | -7.51% | -3.16% |
| field_sleeping_1000 | -6.66% | +4.20% | -3.38% | +7.75% |
| rollback8_mixed_settling_1000 | n/a | n/a | -6.03% | -2.61% |
| rollback8_field_sleeping_1000 | +5.45% | +1.75% | +5.03% | +6.87% |

**Settled-1000 was slower in both pairs:** slopes **+3.37% / +14.74%**, medians
**+1.74% / +7.14%**. Settled-500 medians and sleeping-rollback slopes/medians also
worsened in both pairs. Other cases had mixed outcomes or apparent improvements.
The awake-rollback median changes **−6.03% / −2.61%** did not establish an overall
benefit sufficient to retain the change. **No speedup is claimed.**

Awake rollback used **Flat / Flat / Flat / Linear** sampling for A1/B1/B2/A2.
Flat output has **no Criterion slope**, so both slope comparisons remain `n/a`;
no mean was substituted for a slope and no combined slope claim is made. The
second median pair also crosses sampling modes and should be interpreted
cautiously. Each run's original mean, median, available slope and confidence
intervals are archived separately.

For transparency, actual work collected in each ABBA case is below. Each cell is
**iterations / measured seconds**; these are not the complete invocation wall times.

| Case | A1 | B1 | B2 | A2 |
|---|---:|---:|---:|---:|
| mixed_settling_500 | 1,680 / 2.024 | 1,680 / 2.040 | 1,680 / 2.000 | 1,680 / 2.103 |
| mixed_settling_1000 | 840 / 2.449 | 840 / 2.289 | 840 / 2.446 | 840 / 2.361 |
| mixed_settled_500 | 1,680 / 2.323 | 1,680 / 2.254 | 1,470 / 2.286 | 1,680 / 2.174 |
| mixed_settled_1000 | 840 / 2.554 | 630 / 1.975 | 630 / 2.130 | 630 / 1.896 |
| field_sleeping_500 | 345,450 / 1.960 | 378,630 / 2.013 | 347,130 / 1.951 | 363,090 / 2.068 |
| field_sleeping_1000 | 181,650 / 2.068 | 190,260 / 2.030 | 183,120 / 2.052 | 197,820 / 2.103 |
| rollback8_mixed_settling_1000 | 120 / 2.360 | 120 / 2.210 | 100 / 1.868 | 210 / 4.076 |
| rollback8_field_sleeping_1000 | 21,840 / 2.014 | 20,580 / 1.996 | 20,580 / 2.036 | 21,840 / 2.091 |

Same-binary slope drift among Linear-mode cases in this ABBA sequence ranged
**−8.53% to +5.07%** for baseline and **−2.01% to +14.39%** for candidate.
Together with the earlier baseline drift and code-layout differences, this limits
causal attribution. Rejecting retention does **not** prove the technique always
harms performance. Per-case geometric ratios in `provenance.json` summarize the
two pair ratios only; they are neither uncertainty intervals nor extra runs.

## Correctness and validation limits

Code/math review established the zero-impulse identity: `FP` equality compares the
raw i64 value, `x * 0` is zero even at raw extrema, and the fixed-point rounding
`(0 + 32768) >> 16` remains zero. Every linear/angular delta in the original
zero-lambda application is zero; adding/subtracting it cannot overflow, and the
function has no other side effects. This also preserves debug overflow behavior
for zero lambda; the nonzero path is untouched.

That review is **not executed correctness evidence**. The preserved patch contains
729 zero-impulse combinations and 24 nonzero combinations against a copy of the
original function, but **zero tests executed**.

| Check | Result |
|---|---|
| Baseline/candidate benchmark builds | Passed; production code compiled |
| Two baseline and four ABBA benchmark invocations | Completed, all eight cases each, exit 0 |
| Focused debug solver tests | Compilation stopped by disk guard before tests; neither a test pass nor test failure |
| Native release golden, scratch-history, rollback/Session checks | Not run |
| Strict Clippy | Not run |
| Wasm correctness/performance | Not run; only native target installed, Wasmtime absent |
| Independent read-only review | Verified candidate math, ABBA samples/calculations, input/compiler/profile and binary hashes, CPU affinity, and clean reversion; no native correctness pass established |

The debug command was
`cargo test -p orr_physics3d --lib solver::tests --locked --offline -j1`.
Its 180 s resource wrapper stopped compilation after **28.314 s**, exit **−15**,
when sampled free space reached **1,071,673,344 bytes**, below the
**1,073,741,824-byte** floor. Validation stopped after the performance rejection
and resource abort; release correctness and Clippy were never started. After
owned compiler processes exited, only this attempt's newly created debug cache
was removed with authorization. Evidence, release files and source were retained;
free space recovered to **1,357,987,840 bytes**. No further builds or measurements
were run for this documentation.

## Archive and reproduction of the reported arithmetic

This selected archive is self-contained for checking sample counts, actual work,
point estimates and reported pair ratios:

- [raw-measurements.json](measurements/physics3d-2026-10-06/raw-measurements.json):
  all **48 case/run records** and original **960 aggregate sample pairs**;
  original `sampling_mode`, `iters`, `times` and complete `estimates` objects
- [provenance.json](measurements/physics3d-2026-10-06/provenance.json): source/binary
  identity, settings, sanitized bounds/timestamps, comparisons and original hashes
- [candidate.patch.txt](measurements/physics3d-2026-10-06/candidate.patch.txt): exact
  rejected guard/tests, unchanged bytes and inactive as documentation
- [SHA256SUMS](measurements/physics3d-2026-10-06/SHA256SUMS): this report and the
  three archived files, relative to the measurements directory

`times[k]` is elapsed **nanoseconds** for `iters[k]` iterations. It is not an
individual frame latency. For each case/run, use every recorded sample:

- Actual iterations: `sum(iters)`; measured seconds: `sum(times) / 1e9`
- Normalized samples: `times[k] / iters[k]`; reported median and mean use these samples
- Linear slope through the origin: `sum(iters[k] * times[k]) / sum(iters[k] ** 2)`
- Pair percentage: `100 * (candidate_point / baseline_point - 1)`
- Optional geometric pair summary: `100 * (sqrt((B1/A1) * (B2/A2)) - 1)`

Original bootstrap confidence intervals and standard errors are preserved, not
regenerated. Recomputing point arithmetic from raw arrays is deterministic within
floating-point roundoff; bootstrapping again without the original RNG state does
not promise identical confidence bounds. The archive retained all raw samples.

The original collector manifests were fully hash-checked before packaging:

| Original evidence set | Manifest SHA-256 | Verified entries |
|---|---|---:|
| baseline_repeat | `b1de51e89814ba85de2e32f739979ef25099430904421f318c042588b4bacefe` | 154 |
| zero_impulse_abba | `8fb23022045c6508ddce8ad171947cd79d6ce322a80b6475b2ede25cd83c71e9` | 302 |

Those hashes identify the original collector sets; they are not the checksum of
this selected repository archive. Original per-file hashes are retained for raw
Criterion JSON and selected logs/metadata. Collector binaries, scripts, raw
process/environment logs, cache files and absolute workspace paths are not bundled.
Sanitized/repackaged JSON has its own hashes in the archive manifest; original
hashes are never presented as hashes of sanitized bytes.

The documentation consistency check matched every archived sample/estimate object
to its original, independently recomputed counts, durations, means, medians and
available slopes, and checked the published source-tree mapping. No Cargo,
benchmark, CI, or remote action is needed to verify the archive checksum:

```sh
cd docs/measurements/physics3d-2026-10-06
sha256sum -c SHA256SUMS
```

## What remains for #8

This closes the evidence-recording step only. A useful next authorized performance
attempt still needs phase-level measurements to choose a bottleneck, controlled
paired candidate repetitions with sampling-mode consistency, executed native
golden/scratch-history/rollback checks and strict Clippy, and wasm correctness and
performance evidence. Full Session/network rollback latency and a portable product
budget remain unestablished. Do not close #8 or claim a performance improvement
from this record.
