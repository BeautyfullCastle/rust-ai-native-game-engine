# Gather/mover fusion: bounded, unadopted experiment

## Decision boundary

This single native ABBA campaign is insufficient for general adoption. Sleeping-500 has a bounded improvement signal (slopes −7.107%/−7.617%; medians −4.157%/−4.255%); sleeping-1000 magnitude is unstable against same-binary baseline drift. Awake controls are mixed. No rollback improvement or wake-heavy performance improvement is claimed. Independent math/provenance review is complete; final public-artifact privacy review is pending. This docs-only commit does not change production code or close issue #8.

The local candidate commit was `15416574856d1288c650b788844af1b978006f4d`, tree `b98ca9940498fd61bcbea2106ffd75a61c29f4ed`, compared with baseline `f0162b9222e42d1b74247ab6aeebd9ff65b86ac3`. The candidate identifier is local provenance, not a claim of remote availability. The exact [unadopted patch](unadopted-candidate.patch.txt) is an inactive reproduction artifact, not a shipped optimization. Source and binary hashes are in [provenance.json](provenance.json).

## Scope and protocol

The unchanged eight-case benchmark, fixtures, solver settings and goldens match the [baseline protocol](../../physics3d-rollback-baseline.md). Sleeping cases use 500/1000 dynamic bodies in separated groups warmed for 900 ticks. Awake mixed-pile settling/settled controls use 500/1000 bodies and 90/600 preparation ticks. Single-step cases restore their snapshot once per 100 steps; rollback8 cases restore and step eight times. There are no extra wake-heavy or timing cases.

Rust/Cargo 1.97.1, Criterion 0.5.1, x86_64 native, unchanged bench/release profile (opt-level 3, thin LTO, one codegen unit, debug 1), no flag/profile overrides. CPU0 logical affinity does not establish physical isolation. The visible process namespace cannot exclude other host workloads. Builds used `cargo bench -p orr_physics3d --bench physics3d --no-run --locked --offline -j1`. Each frozen binary ran with `taskset -c 0 <binary> --bench --noplot --sample-size 20 --warm-up-time 1 --measurement-time 2 --nresamples 1000 --save-baseline <unique-label>`, with a unique initially absent Criterion output home. Auto sampling was retained. Exactly A1/B1/B2/A2 ran; no reruns or extra timing cases.

Each build had a 300-second cap; each invocation a 180-second cap. A 1 GiB free-disk guard checked every 250 ms; process/load/affinity snapshots were captured every five seconds. No resource guard fired. The initial inherited affinity preceded taskset; every observed postlaunch affinity was CPU0.

## Preserved build-provenance failure

The fresh baseline build completed in 30.047 seconds. The initial candidate build completed in 0.254 seconds but returned the identical baseline executable. This was caught before any timing and the campaign stopped. Relative dependency paths, timestamp freshness and older candidate source supported a stale shared-target explanation. One explicitly authorized corrective build touched only candidate step.rs mtime, preserving source bytes, hashes and clean Git state. Its log explicitly compiled the candidate crate; the resulting different executable was frozen and verified before and after each candidate run. Corrective build time was 21.546 seconds. No release cache was deleted. Original failed-attempt logs and binaries remain preserved locally; their hashes and scalar results are archived here. The rejected zero-impulse candidate was not used.

## Run bounds
| Run | Wall seconds | Minimum free bytes | 1-minute load range | Exit |
|---|---:|---:|---|---:|
| a1 | 31.800 | 1487011840 | 0.66–0.80 | 0 |
| b1 | 31.547 | 1486585856 | 0.80–0.93 | 0 |
| b2 | 33.050 | 1486159872 | 0.94–0.96 | 0 |
| a2 | 33.564 | 1485733888 | 0.96–0.98 | 0 |

## Sleeping cases
Pair 1 = B1/A1; pair 2 = B2/A2. Negative means lower measured cost.
| Case | Slope pair1 % | Slope pair2 % | Median pair1 % | Median pair2 % | Baseline slope drift % | Candidate slope drift % |
|---|---:|---:|---:|---:|---:|---:|
| field_sleeping_500 | -7.107 | -7.617 | -4.157 | -4.255 | +2.302 | +1.741 |
| field_sleeping_1000 | -15.624 | -1.753 | -14.482 | -3.051 | -12.831 | +1.498 |
| rollback8_field_sleeping_1000 | -0.374 | +1.127 | -0.256 | -0.426 | -3.474 | -2.020 |

## Awake controls
Pair 1 = B1/A1; pair 2 = B2/A2. Negative means lower measured cost.
| Case | Slope pair1 % | Slope pair2 % | Median pair1 % | Median pair2 % | Baseline slope drift % | Candidate slope drift % |
|---|---:|---:|---:|---:|---:|---:|
| mixed_settling_500 | +0.476 | +0.660 | +1.774 | +0.310 | -1.577 | -1.396 |
| mixed_settling_1000 | -2.274 | -4.626 | +0.953 | -3.364 | -1.213 | -3.591 |
| mixed_settled_500 | +6.351 | -3.977 | +5.741 | -1.770 | +3.677 | -6.392 |
| mixed_settled_1000 | -4.043 | +0.067 | -3.274 | +0.621 | -7.966 | -4.023 |
| rollback8_mixed_settling_1000 | n/a | -3.302 | -2.513 | -3.275 | n/a | n/a |

## Per-case original estimates
Milliseconds per benchmark iteration, including each original 95% confidence interval. Sampling mode retained; no mean substitutes for an absent slope. Awake rollback uses Flat in pair 1 and Linear in pair 2: each pair is mode-consistent, but pooling across pairs needs caution. Pair 1 has no slope. Actual measurement durations may exceed the requested two seconds. No samples/outliers removed.
| Case/run | Mode | Slope ms [CI] | Median ms [CI] | Iterations | Measured seconds | Normalized sample CV % |
|---|---|---|---|---:|---:|---:|
| mixed_settling_500/a1 | Linear | 1.219968 [1.198655, 1.249028] | 1.233969 [1.213017, 1.264870] | 1680 | 2.068135 | 4.229 |
| mixed_settling_500/b1 | Linear | 1.225778 [1.201160, 1.251525] | 1.255854 [1.202473, 1.277911] | 1680 | 2.073260 | 6.549 |
| mixed_settling_500/b2 | Linear | 1.208662 [1.194195, 1.222935] | 1.203706 [1.180720, 1.219443] | 1680 | 2.022719 | 3.843 |
| mixed_settling_500/a2 | Linear | 1.200734 [1.184803, 1.215002] | 1.199989 [1.171575, 1.218914] | 1680 | 2.013605 | 3.858 |
| mixed_settling_1000/a1 | Linear | 2.888928 [2.803610, 2.977851] | 2.796289 [2.735786, 2.892861] | 840 | 2.402549 | 7.325 |
| mixed_settling_1000/b1 | Linear | 2.823247 [2.775672, 2.870568] | 2.822947 [2.725860, 2.883264] | 840 | 2.370176 | 5.305 |
| mixed_settling_1000/b2 | Linear | 2.721873 [2.684015, 2.765367] | 2.712763 [2.655724, 2.779682] | 840 | 2.289076 | 4.175 |
| mixed_settling_1000/a2 | Linear | 2.853895 [2.797032, 2.904587] | 2.807206 [2.749272, 2.880742] | 840 | 2.388219 | 6.002 |
| mixed_settled_500/a1 | Linear | 1.282939 [1.274415, 1.297974] | 1.279048 [1.272202, 1.361776] | 1470 | 1.902797 | 4.258 |
| mixed_settled_500/b1 | Linear | 1.364422 [1.347652, 1.377735] | 1.352477 [1.320384, 1.366909] | 1680 | 2.280365 | 2.817 |
| mixed_settled_500/b2 | Linear | 1.277211 [1.267290, 1.289511] | 1.282536 [1.266571, 1.298825] | 1680 | 2.156726 | 15.540 |
| mixed_settled_500/a2 | Linear | 1.330111 [1.292360, 1.384294] | 1.305647 [1.278462, 1.346145] | 1470 | 1.958106 | 7.785 |
| mixed_settled_1000/a1 | Linear | 3.273826 [3.246765, 3.303163] | 3.256490 [3.155285, 3.283089] | 840 | 2.737337 | 3.826 |
| mixed_settled_1000/b1 | Linear | 3.141452 [3.091192, 3.185557] | 3.149877 [3.051925, 3.209473] | 840 | 2.636137 | 7.601 |
| mixed_settled_1000/b2 | Linear | 3.015070 [2.986127, 3.054894] | 3.035417 [2.993667, 3.114009] | 840 | 2.555814 | 8.144 |
| mixed_settled_1000/a2 | Linear | 3.013042 [2.986449, 3.055963] | 3.016680 [2.977386, 3.046105] | 840 | 2.548002 | 4.688 |
| field_sleeping_500/a1 | Linear | 0.005487 [0.005312, 0.005785] | 0.005307 [0.005267, 0.005353] | 367920 | 2.016597 | 6.475 |
| field_sleeping_500/b1 | Linear | 0.005097 [0.005075, 0.005124] | 0.005087 [0.005058, 0.005135] | 382200 | 1.948870 | 1.458 |
| field_sleeping_500/b2 | Linear | 0.005186 [0.005104, 0.005351] | 0.005348 [0.005077, 0.005532] | 367500 | 1.945046 | 9.543 |
| field_sleeping_500/a2 | Linear | 0.005614 [0.005464, 0.005818] | 0.005585 [0.005442, 0.005819] | 371280 | 2.094225 | 17.612 |
| field_sleeping_1000/a1 | Linear | 0.011788 [0.011286, 0.012327] | 0.011525 [0.011296, 0.011743] | 183960 | 2.153815 | 6.228 |
| field_sleeping_1000/b1 | Linear | 0.009946 [0.009847, 0.010099] | 0.009856 [0.009823, 0.009889] | 202230 | 2.007173 | 1.755 |
| field_sleeping_1000/b2 | Linear | 0.010095 [0.009956, 0.010352] | 0.009932 [0.009860, 0.010730] | 191520 | 1.969476 | 9.826 |
| field_sleeping_1000/a2 | Linear | 0.010275 [0.010230, 0.010312] | 0.010244 [0.010198, 0.010298] | 193410 | 1.987207 | 1.370 |
| rollback8_mixed_settling_1000/a1 | Flat | n/a | 19.991709 [19.276996, 20.550300] | 100 | 2.006650 | 6.162 |
| rollback8_mixed_settling_1000/b1 | Flat | n/a | 19.489368 [19.395910, 19.660125] | 100 | 1.970043 | 3.805 |
| rollback8_mixed_settling_1000/b2 | Linear | 18.561189 [18.387518, 18.791079] | 18.400291 [18.289880, 18.642802] | 210 | 3.901593 | 2.314 |
| rollback8_mixed_settling_1000/a2 | Linear | 19.195045 [18.927810, 19.547678] | 19.023236 [18.858402, 19.281482] | 210 | 4.032019 | 3.086 |
| rollback8_field_sleeping_1000/a1 | Linear | 0.094242 [0.093270, 0.095223] | 0.093848 [0.092497, 0.094915] | 19950 | 1.881180 | 2.270 |
| rollback8_field_sleeping_1000/b1 | Linear | 0.093889 [0.092834, 0.094818] | 0.093608 [0.092020, 0.095422] | 21630 | 2.030019 | 2.989 |
| rollback8_field_sleeping_1000/b2 | Linear | 0.091993 [0.089865, 0.094566] | 0.089826 [0.089266, 0.092454] | 22680 | 2.081977 | 3.766 |
| rollback8_field_sleeping_1000/a2 | Linear | 0.090968 [0.090230, 0.091832] | 0.090210 [0.089796, 0.090648] | 22050 | 2.002679 | 1.710 |

## Correctness and unmeasured work

The implementation owner reported five private gather tests, three sleep/wake integration tests, ten golden tests and strict Clippy passed on the candidate. These were separate checks, not benchmark assertions or checks rerun during this campaign. Existing tests include impulse/contact island wake, support removal, sleep/wake rollback, and direct gather island/velocity wake paths. They do not measure sustained wake-heavy performance. No new benchmark cases, dependencies, goldens or workflow changes were made.

No wasm correctness/performance or full Session/network rollback latency was measured. Binary-layout effects cannot be separated from code effects in these timings. Point differences and confidence intervals are not universal noise thresholds, causal proof, worst-case latency bounds or evidence of broad gain.

## Reproduction and archive

[raw-measurements.json](raw-measurements.json) preserves all 32 original sample/estimate objects and all 640 aggregate samples with original modes, estimates and confidence bounds. Its bytes match the local extracted numerical archive exactly. [provenance.json](provenance.json) includes complete estimates, sample distributions, coefficients of variation, work counts, pair ratios, same-binary drift, source/fixture/binary hashes and bounded scalar run/build results. No raw process arguments, environment variables or private paths are included. [transformation-manifest.json](transformation-manifest.json) records those omissions and the original local-file hashes. [SHA256SUMS](SHA256SUMS) hashes the five content files.

For each sample, normalized cost is times/iters in nanoseconds. Actual work is sum(iters), measured seconds sum(times)/1e9. Linear slope is sum(iters*times)/sum(iters squared). Pair percent is 100*(candidate/baseline−1). Means, medians and available slopes were recalculated from all samples at 1e-10 relative tolerance. Original bootstrap confidence bounds were preserved, not regenerated. Pair geometric summaries in provenance describe two point ratios only; they are not additional runs or uncertainty intervals.

Verify without Cargo or benchmarks: run `sha256sum -c SHA256SUMS` from this directory.
