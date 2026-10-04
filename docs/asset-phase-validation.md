# Asset phase validation: Windows single observations

2026-10-03 UTC, [#24](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/24).
The authorized support stage completed two preparation builds and one campaign
of nine serial child observations. All exited zero; independent process,
output-contract and archive reviews found no blocker. The
[final owner handoff](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/31#issuecomment-5974704328)
records the results and local evidence. This is not completion of the whole
asset epic or a product memory/performance budget.

## Measurement identity and environment

- Measured commit: `ba85154a08e91fdeeaa141be3d47fc82af536922`.
- Measured full tree: `f65c395ed95124e945d72bed6a79dcf35ea33c4b`.
- Base: `6762f85af8d606201226f93e199bf3473b418d9c`.
- Driver: [`tools/asset_phase_memory_probe.py`](../tools/asset_phase_memory_probe.py),
  60172 bytes; SHA-256 `9c3396231ef471e31e58ff43d0cb66a9a9c70ffeeaba38c16ce887b3d01c4a10`.
- Windows 11 Education 10.0.26200; Intel Core i7-14700, 20 cores/28 logical
  processors; Balanced power plan; Python 3.13.7.
- Rust/Cargo 1.97.1, LLVM 22.1.6, `x86_64-pc-windows-msvc`, release profile.
  Cooker default features; fixture libtest with `audio`.
- Fresh compile target; existing Cargo registry/git dependency caches inherited
  without eviction. The selected Rust/Cargo flags and wrappers were unset.
  Repository workload coordination was exclusive; desktop/app services remained.
  This was not whole-CPU isolation or an OS filesystem-cache cold/warm control.

Source, fixture, plan and executable fingerprints matched before/after and at
review, with a clean worktree. This document and the §7 wording were added
**after** measurement. The document publication commit is recorded in its PR;
its CI is separate from execution at the measured commit above.

The earlier driver at `1c4ffca0a2b5596d99d50182b7b40ab946f4583c` passed 23
focused checks. The measured driver then passed one focused exact-test-name
delta check and Ddang's source cross-review. These are separate source bases,
not a newly executed full 24-check suite.

## Preparation and fixed workload

The preparation generator created two valid input variants, `max_index` (A)
and `max_source` (B). Each has 81 input files totaling exactly 1048576 bytes:
1024 index records = 64 motion + 16 distinct PCM + 944 tombstones. Each has 80
distinct cooked payloads/cache keys; B's last processed source is 65536 bytes.
Padding moves bytes between the index and sources without changing canonical
content. A/B raw source/cache-key differences are exactly 16 records (IDs
1–15 and 80); 64 records are unchanged.

Preparation ran these two serial commands into a fresh `build` directory:

```sh
cargo build --release --locked --offline -p orr_asset_cook --target-dir <fresh>/build --message-format=json
cargo test --release --locked --offline -p orr_asset_fixture --features audio --lib --no-run --target-dir <fresh>/build --message-format=json
```

Both Cargo parents exited zero. Their outer driver interval was
23:26:46.3076864–23:27:17.1985460 UTC (30.8897506 s). The two preparation builds,
fixture generator and metadata processes are outside the nine observations;
two Cargo commands do not mean only two internal OS processes. Captured Cargo
parent memory excludes rustc descendants and is not a build-tree peak.

Cargo JSON fixed the exact executables, with copies and fingerprints retained:

| Executable | Bytes | SHA-256 |
|---|---:|---|
| `orr_asset_cook.exe` | 487424 | `7b88bc529ed85b28fadd5ff3fe251b8e7e88c81f4984cd8127cbacf64d6390aa` |
| `orr_asset_fixture` audio libtest | 1577472 | `168cab0ab489e82dd67540d48d21ac7f337b0f64bd31c3b7c550b4bfff722b11` |

## Nine completed observations

For each variant: cook once with an empty dedicated asset cache, cook to a new
output using that cache, check the existing cold output without a cache, then
inspect the cold output. The ninth child is the existing exact runtime test.
Each case ran once. The outer campaign interval was
23:29:44.0524367–23:30:00.2323629 UTC (16.1788359 s), including supervision and
validation between children. It is not the sum of private phase execution times.

`WS` below is sampled WorkingSetSize; `PeakWS` is the maximum OS-reported
lifetime PeakWorkingSetSize read at those observations; `PrivateUsage` is the
sampled process field. All are whole-child values, including setup/allocator
overhead. Command wall includes process setup, observer work and drain.

| Case | Command wall ms | Max observed WS KiB | Max reported PeakWS KiB | Max observed PrivateUsage KiB | Samples |
|---|---:|---:|---:|---:|---:|
| A cold cook | 237.5498 | 8092 | 8092 | 3780 | 18 |
| A warm cook | 120.2771 | 7940 | 7940 | 3676 | 10 |
| A check | 46.5142 | 7836 | 7836 | 3616 | 3 |
| A inspect | 35.7562 | 5000 | 5000 | 868 | 2 |
| B cold cook | 188.9563 | 6960 | 6960 | 2672 | 17 |
| B warm cook | 116.3464 | 7056 | 7056 | 2776 | 10 |
| B check | 44.9313 | 6856 | 6856 | 2668 | 3 |
| B inspect | 53.6229 | 4996 | 4996 | 868 | 2 |
| Runtime boundary | 88.0117 | 6692 | 10948 | 4148 | 2 |

All nine distinct owned children exited zero, drained, were naturally reaped,
and had their held process handles closed. The 27 stdout/stderr/memory streams
and nine process records matched retained byte counts/hashes, with no discarded
bytes. The post-run process census at 23:31:00.7153667 UTC found no related
process; the lane was returned. This is a snapshot, not future quiet-lane proof.

There were 67 Windows PSAPI observations. The target interval was 10 ms;
actual intervals were 10.0734–10.6572 ms. Short transients can be missed. The
runtime's sampled maximum WS (6692 KiB) differs from its reported lifetime
PeakWS (10948 KiB). PrivateUsage is not private heap or an internal-stage
allocation measurement. Linux `wait4.ru_maxrss` is unsupported/null for this
Windows capture; no new Linux, macOS or ARM observations were performed.
One observation per case supports no median, p95, causal speedup or ratio.

## Output and admission checks

Each variant reported cold/warm/check cache hits of 0/80/0, with all 80 ORAC
headers/payloads/cache keys/digests validated. Typed ORAM manifests held 64 SIM
and 16 VIEW records in order. Cold/warm output bytes, including provenance,
matched within each variant; check left the output unchanged. Inspect stdout
matched the literal bundle inspection JSON. A/B canonical objects, manifests,
generated SIM source and inspect output matched; raw provenance differed.

| Canonical manifest | SHA-256 |
|---|---|
| SIM | `487c7ccc30e8fa1a1c153b7477f2f6bfc87b104ec9958f1a85669db52b612a30` |
| VIEW | `9d827363796e8419af63a8966caac95b1828f400d025ad8dbcebd2abc5760922` |

The runtime child used
`audio::tests::decoded_budget_exact_max_and_one_frame_over_are_checked_before_conversion`
with `--exact --nocapture --test-threads=1`: the named test was `ok`, with
1 passed, 0 failed, 0 ignored and 26 filtered. It covers exact 4 MiB retained
decoded admission, one-frame-over rejection before conversion and the embedded
required digest-mismatch case. Its max bank has 11 records/524288 frames,
1048664 cooked bytes and a 632-byte manifest. It does **not** construct a
successful max-bank mixer. The libtest summary's `0 measured` counts benchmarks,
not the observer's completed runtime child.

The cooker fixture's 16 × 48000-frame clips produce 1536128 VIEW bytes (plus
512 SIM bytes). Decoding all those clips would require 6144000 retained stereo
bytes, exceeding runtime's 4 MiB admission policy. The cooker bundle is not
proof of runtime admission of that full decoded bundle. Each cache touched
1543680 bytes including 80 × 88-byte headers; this is not a global disk-cache
cap or garbage-collection guarantee.

## Ownership, bounds and unmeasured scope

Cooker overlap includes index bytes/parsed entries, the current source Vec,
accumulated distinct cooked payloads and current cache payload/copies. Previous
source Vecs drop at each iteration end; the entire cache directory is not loaded.
Check overlaps expected outputs with the current actual file; inspect overlaps
manifests, the current object and inspection records.

Runtime preload overlaps borrowed cooked bytes/manifest, retained decoded Arc
samples and one current conversion buffer. `PreparedFixture` owns binding and
checksums; source JSON/cache are not runtime inputs. A mixer is normally created
after bank preload; this boundary test does not construct a successful maximum
mixer. **A single-process simultaneous source/cache/cooked/decoded peak remains
unmeasured.** Separate phase peaks must not be added to claim that quantity.
Internal-stage heap/peak, maximum-mixer memory, RSS ceiling and hard preemption
also remain unmeasured. Existing historical Linux representative observations
in [AUDIO_VALIDATION.md](../crates/orr_asset_fixture/AUDIO_VALIDATION.md) remain
separate evidence.

The input/record/manifest/retained-decoded policies in
[asset-pipeline-v1.md §7](asset-pipeline-v1.md#7-하드-상한과-실행-예산) were preserved.
The two fixtures exercise the stated finite input bounds; they do not bound
arbitrary accumulated disk caches or all possible authoring workloads.
Managed raw evidence was 864031 bytes under 64 MiB; fixture/cache/output logical
bytes were 11527116 under 32 MiB. Intermediate build products, filesystem
overhead and outer observer files are separate. Logical/staging accounting is
not an enforced disk/heap/RSS quota.

Preparation had a 900 s observed watchdog per command; observations had 30 s
each. First unexpected failure prevents future launches and preserves attempts
and raw prefixes/discard accounting. A live child is drained and awaited without
automatic kill or replacement; pending natural exit is not bounded or guaranteed
by the watchdog. All actual children passed, so this campaign does not establish
real failure-stop activation or forced cancellation. No extra sample/rerun is
implied by this record.

## Retained evidence and tool modes

The primary local archive directory is
`E:/Projects/Fork/rust-ai-native-game-engine/target/haneul-24-phase-implementation/`.
The original run remains in the measurement worktree at
`target/asset-phase-ba85154-20261003T232545Z/`. These are retained local evidence,
not committed or automatically downloadable public artifacts. Original binaries
and the ZIP are not added to this PR. The owner handoff above provides a public
locator for this preservation record.

| Evidence | SHA-256 |
|---|---|
| Run `report.json` (136206 bytes) | `e8f4f6f89e93f78014929abde4b7ffbbda3aeaa4b394a394e59f5fc490d9a69a` |
| `build-report.json` (7457 bytes) | `9e9c0824c8a5914167b4c329036c4567d11e0c93ce08a3463b8adf495f8f6b6f` |
| Run `plan.json` | `5ad19f0a2cc26b49566e6087ce98c216b8a8d422c78ea4311f960b5c5b621e70` |
| `actual-phase-observation-summary.json` | `b2642a6a683ab144a44299909f7fd31edea42831c4b4d46042bcb72bb8633dc9` |
| `final-phase-completion-proof.json` | `0651ccc8ca6598a18432d2ae705c725156010ec53d804ebcb42fbe8c5afe2f30` |
| `actual-ba85154-phase-evidence.zip` | `ad4c694d457af4c5f1b074d4038ed06b9c5ab8bc71c347b94ccc774a9979097f` |

The ZIP has 4684999 bytes/722 entries (711 run files + 11 provenance files),
14527188 uncompressed bytes, matching CRC/index/original file hashes. It retains
two executable copies, source, plan, inputs, cache, bundles and raw evidence;
intermediate Cargo build products are excluded. Packaging is not measurement.
Separate independent prebuild/process/contract/archive reviews are linked by
their hashes in `final-phase-completion-proof.json`; no normal historical whole
CI/raw audit was repeated to create this document.

The standard-library driver has mutually exclusive `--self-test`, `--prepare`,
`--build`, `--run` modes. `--prepare --root <clean-worktree> --expected-head
<approved-exact-SHA> --output <fresh-worktree-target>` creates an immutable plan;
`--build --output <same-output>` builds the two executables once; `--run --output
<same-output>` consumes that preparation once for the fixed nine serial children.
Source/fixture/executable fingerprints and first-start markers reject reuse or
drift. A future campaign requires its own agreed source/environment/quiet lane
and fresh output; this document does not authorize rerunning the retained output.

## Follow-up decode allocation observation

The minimum follow-up was authorized in
[su5978333338](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/30#issuecomment-5978333338).
Its source base is `569fa578f82c440ab783a4937d0e84e223bb868b`. Local release
compilation, focused contracts, mutual source review and the two approved actual
observations completed on 2026-10-04. The earlier nine observations above remain
separate evidence. This follow-up measures the requested Rust allocation layouts
of real bank preload; it does not measure System's private heap or process RSS.

The default-off `decode-memory-probe` feature exposes an opaque safe bank owner
around the same strict `PreparedFixture`/manifest/object/4 MiB admission and
real preload helper. It creates no mixer. A callback marks each record immediately
before PCM conversion, after whole-bank admission. The library still forbids
unsafe code. A separate integration-test binary wraps `System` through
`GlobalAlloc`; no dependency or default audio/simulation path is added.

Fixture generation, package admission, caller-owned input allocation and
observer bookkeeping precede the guarded interval. The fixed event buffer holds
4096 allocation/deallocation/reallocation events, including failed reallocations;
serialization happens after recording, with a 1 MiB result limit. Allocations
alive when the bank returns are classified as retained lifetimes. Lifetimes
freed before that boundary are transient. Replaying the ordered events gives
retained-only, transient-only and simultaneously live combined peaks; the
transient peak is not calculated by subtracting final retained bytes from a
whole peak. Reallocation and pointer reuse must preserve distinct lifetime
identity. Dropping the returned bank is observed separately to establish the
recorded cohort's release. Overflow, unrecognized pointers, foreign-thread
events or incomplete accounting invalidate numeric conclusions and preserve
an explicit error/unknown result.

The approved actual scope is one fresh serial child for the 11-record,
524288-frame, exactly 4 MiB retained-sample bank and one for the same fixture
with one additional frame. The latter must return `BudgetExceeded` with zero
conversion-start callbacks; it does not establish zero allocations during
preflight. Strict custom package preparation uses the binary-pinned SIM table
and a reviewed view release digest. The existing max/+1 test is unchanged.
Synthetic accounting contract tests do not count as these actual observations.
Before launching them, freeze source/fixture/executable/environment and obtain
a fresh #28 lane after focused checks and Haneul's narrow source review.
The 30 s observation watchdog does not kill or preempt a child; first failure
stops subsequent launches, and actual exit/drain/reap precede lane return.

The reported byte quantities cover Rust allocator-requested layouts in this
bounded helper interval at recorded allocator-call boundaries. A realloc records
its returned old-to-new layout transition; internal System scratch or moved-block
overlap inside that call is not visible. They exclude allocator internals, native allocations,
stack, device/GPU memory and process RSS. Borrowed cooked/manifest byte counts
and caller Vec capacities are separate inputs, not decoder-owned allocations.
OS WorkingSet/PeakWorkingSet/PrivateUsage may complement the trace but cannot
identify the decoder's private phase; samples may miss transients. Neither
peak subtraction nor summing separate maxima yields a four-way simultaneous
peak. No max-bank mixer, ratio, p95, product budget or RSS ceiling is inferred.

The original source/cache/cooked/decoded single-process condition was
**unfulfilled**. The owner adopted reporting according to actual cooker and
runtime lifetimes, preserving that history: runtime has no source/cache input
path. This change does not create or measure such a path. The new results below
preserve that distinction; final epic completion remains a separate owner decision.

Focused validation used Rust/Cargo 1.97.1, x86_64 MSVC release, locked/offline,
with an existing dependency target cache. The default library had 18 passing
tests. The diagnostic-feature library had 26 passing tests and one existing
informational timing test ignored. Three synthetic ledger contracts passed;
the two actual observation cases remained ignored. Strict all-targets Clippy
passed in no-default, audio-only and diagnostic-feature configurations.
The diagnostic configuration initially failed on two nested conditional lint
errors; a minimal conjunction change preserved the accounting order, then its
release compile, three synthetic contracts and strict Clippy passed. The earlier
failure raw and source remain preserved. These checks do not constitute the two
actual observations, process memory measurements, or a max-bank mixer test.

### Measured identity and execution

The measured clean source was
`fea653ac2ba09e9a6c23bf099b124fd2a912dafc`, tree
`15a36228caf08198d513a6d15c42ba91f0479b6d`, at the source base above. The three
executable-source files are unchanged by this results document. A subsequent
publication commit or its synthetic integration CI has a different identity;
neither is presented as the measured source.

[Haneul source review5978968592](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/31#issuecomment-5978968592)
reported no blocker. Fresh full #28/owners/remote and relevant-process census
preceded [lane claim5979059875](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/28#issuecomment-5979059875).
The two exact selectors were run once each with `--ignored --exact --nocapture
--test-threads=1`, in max/+1 order, using the already compiled immutable release
binary. No warmup, retry, extra case or new compilation was performed.

The environment was Windows 11 Education 10.0.26200, x86_64 MSVC, Intel Core
i7-14700 (20 cores/28 logical processors), Rust/Cargo 1.97.1, release with
`decode-memory-probe`. The earlier compilation reused a trusted dependency
target cache and was not cold. CPU load at the freeze was 63%; the machine was
not isolated. These are allocation observations, not elapsed-time benchmarks.
No OS memory sampler, mixer, audio device or GPU was used.

Outer PID9032 ran from 10:33:51.1672016 to 10:33:52.1868357 UTC. Max PID24236
ran from 10:33:51.3710392 to 10:33:51.5469085; +1 PID24104 ran from
10:33:51.8580004 to 10:33:51.9334218. Both exited naturally with code0 and one
test passed/zero failed/zero ignored. The processes did not overlap. Raw streams
were drained, held handles closed and source5/fixture6/binary hashes unchanged.
Postcensus at 10:34:13.7026387 had no relevant process; actual lane return is
[5979071617](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/28#issuecomment-5979071617).
Neither the 30s watchdog nor the observed raw-output threshold was triggered.

### Actual allocation results

All byte values below are requested Rust layouts at recorded allocator-call
boundaries from preload entry to return. Both traces were complete, with zero
flags and zero foreign-thread calls.

| Observation | Max bank | +1 frame rejection |
|---|---:|---:|
| Distinct PCM records | 11 | 11 |
| Stereo frames | 524288 | 524289 |
| Manifest bytes | 632 | 632 |
| Cooked payload bytes | 1048664 | 1048666 |
| Retained-sample budget bytes | 4194304 | 4194312 (rejected) |
| Result | `ok` | `BudgetExceeded` |
| Conversion-start callbacks | 11 | 0 |
| Recorded events | 46 | 0 |
| Allocation/deallocation calls | 23/23 | 0/0 |
| Reallocation/failed-reallocation calls | 0/0 | 0/0 |
| Retained peak/bytes live at return | 4198880 | 0 |
| Transient peak | 384000 | 0 |
| Simultaneously live combined peak | 4553184 | 0 |
| Recorded bytes/allocations after bank drop | 0/0 | 0/0 |

At max return event34, twelve lifetimes remained: the bank Vec and eleven
decoded Arc allocations. Their requested layouts include 4576 bytes beyond the
4194304 sample bytes. The ordered trace records each temporary conversion Vec
overlapping its new Arc before the Vec is freed. The largest transient occurred
at a different point from the combined peak: **4198880 + 384000 is not the
observed combined peak**. Peaks are computed from simultaneous lifetime states,
not by adding separate maxima or subtracting final retained bytes. Drop events
34..46 establish release of the recorded cohort and do not extend the preload
peak interval. This actual trace had no reallocations; realloc handling has
synthetic/source evidence only.

The +1 result records zero allocation events and zero conversion callbacks in
the guarded preload interval. Fixture construction and strict package preparation
were outside that interval, so it does not claim zero preflight allocation.
Caller-owned manifest and cooked payload capacities remain separate input sizes.
The original four-data simultaneous-process condition, System internal scratch,
native/private heap, RSS and successful max-bank mixer remain unmeasured.

### Retained new evidence

The local-only directory is
`C:/Users/hot41/.codex/worktrees/ddang-24-decode-memory/rust-ai-native-game-engine/target/ddang-24-handoff/actual-two-20261004T1034Z/`.
Its label is a directory name; exact execution timestamps are given above.
The 6003B max stdout and 2722B +1 stdout each contain one bounded result record;
both stderr files are empty. Executables and unnecessary raw files are not
published with this document.

| Evidence | SHA-256 |
|---|---|
| Observer source42934B | `191c22ea65279ed941368d78380473b36a8d9579b30fe6b4d40bcece91a33887` |
| Safe adapter source10578B | `a0d2bb6e722db037458a24ccd2453ecfe28cf4562ad7b8f822331ba7ce2b91f6` |
| Immutable release executable1311744B | `64bbebfb4b8f4149ba597c170259cd01a814a337cd459d39e7067a86e2011dab` |
| Source/fixture/binary/environment freeze | `05014381873e599eac0f89aba204abd2d0d314877796b85827022d9234368f26` |
| Two-case report16746B | `381d262488fdbb54056399fb8e0ec9071d1d7f7cb507272e43ed9842e47e0741` |
| Max stdout6003B | `1fc9e1417a26b0176db343ce22edb6a1c2238c4ba95a87bb47371629d48fdf3f` |
| +1 stdout2722B | `bda28210a2eb5bd4e2258e0292cb7b936d953216eca981ba052d4737ed87bfec` |
| Each empty stderr | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
