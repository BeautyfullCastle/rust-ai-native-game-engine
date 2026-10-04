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

## Follow-up decode allocation observation (implementation in progress)

The minimum follow-up was authorized in
[su5978333338](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/30#issuecomment-5978333338).
Its source base is `569fa578f82c440ab783a4937d0e84e223bb868b`. Local release
compilation and focused contracts completed on 2026-10-04; mutual source review
and the two actual observations remain pending. The earlier nine observations
above remain separate evidence.

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
path. This change does not create or measure such a path. New actual results,
exact measured source identity and raw hashes will be recorded here after the
approved observations; final epic completion remains a separate owner decision.

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