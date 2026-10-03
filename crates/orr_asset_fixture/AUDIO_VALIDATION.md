# Focused validation: opt-in asset audio / CI integration

Date: 2026-10-03 UTC. Base: `b51f9b1e7f2f4b24413e924573d88a60cfe63316`.
Linux x86_64, AMD EPYC 9V74, rustc 1.99.0 (`b940084d7`, LLVM 23.1.1).
Kira 0.12.5 and the offline dependency graph were already cached. No new
third-party package, native audio backend, graphics stack or target was installed.

## Passed locally

Commands ran with the pinned toolchain environment, `CARGO_BUILD_JOBS=2`, offline
resolution and `CARGO_TARGET_DIR=/tmp/orrery-cooker-target`:

```sh
cargo test --offline --locked --release -p orr_asset -p orr_asset_cook -p orr_asset_fixture -p orr_audio --features orr_asset_fixture/audio
cargo test --offline --locked --release -p orr_asset_fixture --no-default-features
cargo test --offline --locked --release -p orr_session --test arena
cargo clippy --offline --locked -p orr_asset -p orr_asset_cook -p orr_asset_fixture --all-targets -- -D warnings
cargo clippy --offline --locked -p orr_asset_fixture -p orr_audio --features orr_asset_fixture/audio --all-targets -- -D warnings
cargo run --offline --locked --release -p orr_asset_cook -- cook --index assets/fixture_v1/index.json --out assets/fixture_v1/cooked --root a_0000000000001001 --root a_0000000000002001 --check
cargo fmt -p orr_asset_fixture -- --check
git diff --check
```

Results: asset core 13 tests + 1 compile-fail doctest; cooker 28 tests;
audio fixture 26 tests + 4 doctests (one informational benchmark normally
ignored); audio-free fixture 18 tests + 4 doctests; existing Kira PCM suite 14
tests; existing session Arena/replay suite 11 tests. All passed. Audio fixture
was also tested in the debug profile. Final strict clippy passed with and without
the audio feature; no default-game golden was edited.

Dependency-tree inspection with `cargo tree --offline --locked --edges normal`
confirmed the default fixture has no audio/bridge/Kira/cooker/sample/graphics
edge. Its audio feature adds the offline audio/bridge graph, with no CPAL, cooker,
sample or graphics dependency. There is no reverse dependency into simulation.

The existing simulation float-type guard (9 libraries and FP interop) and all 8
POD guard fixtures passed. An initial auxiliary guard invocation omitted the
intended target directory and reached the nearly-full workspace disk limit. Its
81 MiB of new local build artifacts were moved to a separate tmpfs target;
only that task's artifacts were moved. Retrying with the explicit tmpfs target
and offline mode passed. No source workaround or policy relaxation was used.

Workflow YAML parses; the required aggregate dependency set and existing
workspace-release/clippy, native-audio, graphics/browser and default-golden
commands remain intact. New steps cover native core/cooker/strict fixture,
read-only cooked reproduction, Linux/Windows offline audio, WASI and WASI SIMD
fixture execution, Android fixture execution, and browser-target audio-free
fixture compile-only checks. Their remote execution is still pending.

## What the new tests establish

- Real Kira output from validated cooked PCM is nonzero, finite, bounded and
  equal in both stereo channels; consumed caller buffers can be dropped
- One atomic poll per real bridge tick: all 300 checksums and complete fixture
  event payload/order match audio-on, audio-off and the admitted scalar runner
- Predicted/verified and late verification start once; cancellation fades to
  silence; ordered canceled→predicted replacement remains audible
- Seek, branch, pause/resume, resync, disconnect, session restart and owner-source
  replacement reach the existing EventAudio lifecycle logic unchanged
- Missing objects/hash mismatch fail required mode and expose a muted reason in
  auto mode. Off skips decode/mixer. Strict release preparation still rejects a
  missing view artifact; these policies do not create a simulation/replay bypass
- 32 voices and 4096 history entries remain bounded; stale retired keys cannot
  restart a sound. An unsupported cue never maps to another asset
- Exact 4 MiB retained decoded samples (524288 stereo frames across 11 clips)
  are accepted; one additional frame is rejected before sample conversion
- 16 view records and one 48000-frame clip are admitted; record/frame +1 is
  rejected. Byte-budget arithmetic checks exact maximum, +1 and usize overflow
- Generic manifest 128 KiB and cooked 2 MiB guards are distinct from the tighter
  current v1 schema maxima: 912 manifest bytes and 1536128 cooked bytes. Such
  generic maximum-size byte strings are not falsely treated as valid packages

The whole replay allowlist, parsed release fields, nonzero identity, strict
keyframe validation and exactly 300 recorded checksums remain unchanged. There
are no new malformed-replay execution, panic or abort tests.

## Informational timing baseline

```sh
cargo test --offline --locked --release -p orr_asset_fixture --features audio --lib measure_asset_tick_and_preload_baselines -- --ignored --nocapture --test-threads=1
```

Three fresh process runs, 300000 ticks per case each, alternating order:

- Asset-backed tick: 64 / 77 / 66 ns per tick
- Same registered state/input/events with constant raw-speed baseline:
  61 / 63 / 65 ns per tick
- 64-entry manifest typed-ref plus table lookup: 12 / 13 / 12 ns per lookup
  over 1000000 lookups each
- Embedded validated preload plus offline mixer construction: 93 / 90 / 83 µs

The comparator verifies every tick's checksum and event first. Both systems have
matching test-only tick-counter instrumentation. Setup/checksum work is outside
the timed loops; input/event allocation and Simulation::step are inside. The
64-entry microbenchmark covers the maximum table separately from the one-record
actual fixture. These are local noisy observations, not latency guarantees,
whole-engine budgets, evidence of zero allocations in an entire tick, or CI
performance thresholds.

## Peak memory observations

Fresh release executables, one test thread, Linux `wait4` child `ru_maxrss` in
KiB. A tiny C fork/exec/wait4 wrapper was used rather than measuring Cargo or
inheriting a Python launcher's higher RSS floor. Equivalent measurements can be
repeated with `/usr/bin/time -v` around the test executable emitted by
`cargo test --no-run --message-format=json` and the following exact test names:

- Audio-free `tests::new_fixture_exact_baseline_schedule_cues_and_closed_replay_roundtrip`: 1988 KiB
- `audio::tests::one_atomic_poll_per_tick_preserves_every_fixture_checksum_and_event`: 3180 KiB
- `audio::tests::decoded_budget_exact_max_and_one_frame_over_are_checked_before_conversion`: 7804 KiB
- Cooker `cook --check` on the checked-in fixture: 1328 KiB
- Fresh cooker output/cache run: 1372 KiB (0 cache hits)
- Second output with the same cache: 1380 KiB (2 cache hits)

Cold/warm output manifests matched; the existing cooker tests also compare all
canonical artifacts across cache states. The maximum preload case holds the
actual 1048664 cooked input bytes, 632-byte manifest and 4194304 decoded sample
bytes together, plus validation/conversion/test overhead. Source JSON and cook
cache are not runtime inputs and never coexist with decoded samples in this
adapter; the separate cooker phase is measured explicitly. This is not a
maximum-size authoring/cache stress benchmark or an allocator-exact heap bound.

The embedded bank retains 57600 decoded sample bytes from a 14408-byte object
and 72-byte manifest. The 4 MiB cap is retained stereo data, not process memory:
one in-progress Clip conversion can temporarily retain an additional sample Vec
(up to 384000 bytes), and the mixer, borrowed inputs and allocator add overhead.
RSS measurements are observations, not hard memory guarantees.

## Review and remaining limits

Independent read-only review found no P1/P2 adapter or workflow issue. Its missing
validation-document link was resolved by this record. The final independent
document/source/benchmark pass found no remaining P1/P2 issue and approved a
local commit; the reviewer did not execute Cargo.

Only Linux x86_64 ran locally. No Windows/macOS/ARM/WASI/browser/Android result is
claimed. Full workspace release/clippy, the graphics-dependent Arena sample
integration test and native-audio compilation were not run in this constrained
lane; their existing CI checks were preserved. No device was opened and no
physical listening/playback success is claimed. No streaming, spatialization,
browser audio or production session/simulation API was added. Parent issue #24
and final #35 integration remain open until the required remote checks pass.
