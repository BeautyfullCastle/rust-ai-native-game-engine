# Static asset fixture v1

Opt-in implementation of issue #34 and `docs/asset-pipeline-v1.md` §9.1.
This crate adds one dedicated game and golden; it changes neither Arena/PhysGame
nor production simulation/replay APIs, ORRF 2, ORRP 3, or deployment-manifest v1.

## Fixed contract

- Game `asset_fixture_v1`, 60 Hz, one player, seed 1, 300 ticks
- One entity: POD `Motion { profile: AssetRef }` and `Position { x: FP }`
- Compiled cooker table uses motion `a_0000000000001001`, speed raw 4096
- Input is a four-byte POD i32 axis: +1 at 1–120, 0 at 121–180, −1 at 181–300
- x(120) = x(180) = 7.5 (raw 491520); x(300) = 0
- POD `Impact { cue: 1 }` occurs once at ticks 1 and 181. Its view asset is
  `IMPACT_ID`, `a_0000000000002001`
- Every tick 1–300 has exactly one checksum; keys are 60/120/180/240/300

`PreparedFixture::embedded()` checks the reviewed cooker output and the separate
`fixtures/release.json`. `prepare(bundle, binding)` accepts borrowed package bytes,
not filenames, and verifies the same compiled sim identity before tick zero.
After admission it retains only validated release identity. Simulation reads the
compiled immutable table and never caller-owned asset bytes.

```rust
use orr_asset_fixture::PreparedFixture;
let fixture = PreparedFixture::embedded()?;
let recording = fixture.record()?;
let report = fixture.verify(&recording.replay)?;
assert_eq!(report.checksums_checked, 300);
assert_eq!(fixture.seek(&recording.replay, 120)?.x.raw(), 491520);
# Ok::<(), orr_asset_fixture::Error>(())
```

`start()` returns a bounded fixed-schedule runner. `advance()` returns only
scalar state and events and refuses the 301st step without changing state.
`record()` accepts no inputs or debug commands. There is no public game type,
mutable Frame/Simulation, arbitrary restore, expected-identity override,
allowlist bypass, branch recording, or hotpatch API.

## Release and package checks

The dedicated raw game-code ID is `11193926291457` (`0x0a2e4a001001`), and its
ORRF-2-bound build ID is `10434068928185386027`. The full compiled sim SHA-256 is:

`1679621b8296cea373c127f397f519dcbfae2d3e7602550f064909b614a31a3a`

The release file has exactly the seven `orr.asset-release/1` fields specified in
the ADR. Its strict parser rejects unknown/duplicate fields, numeric or
noncanonical decimal-string IDs, zero/overflow IDs, wrong frame/build identity,
and non-lowercase/non-64-character digest text. Identity compares parsed fields,
not the JSON representation. `ReleaseIndex::admit` rejects a `(game, raw ID)`
reused with another sim digest. It is an in-memory release-owner ledger: callers
must populate it from retained release bindings and preserve those bindings.
There is no automatic file index or persistence service.

Preparation checks manifest schemas/domains/budgets and complete hashes, every
referenced object's actual length/hash, required cue asset, globally distinct
sim/view GUIDs, and the canonical manifest re-encoding of the compiled table.
Motion and PCM16 header/length checks use integers only. A package cannot contain
missing, ambiguous, or unreferenced objects. View float conversion, clip
ownership and explicit offline-audio policy are opt-in through `audio` (below).
Physical-device output is not provided by this fixture.

Same GUID plus different motion bytes requires a new sim digest, newly assigned
raw game-code ID and rebuilt binary. The old binary refuses it before a tick.
View-only content may change with the same sim/build identity, provided the new
package's view digest, object and PCM header validate. The replay admission gate
binds gameplay identity, not the original audiovisual release: retain each
recording with its original binding, manifests and objects to reproduce the
original presentation. Do not substitute a new release for a missing original.

These hashes provide integrity checks, not signatures. The u64 build identity is
a compatibility label, not a hash of all game code. There is no networking or
late-join guarantee and no runtime external sim-table replacement.

## Closed replay boundary

Before parsing, input is limited to 1 MiB and its **whole compressed ORRP file**
SHA must match the source-controlled closed allowlist. The v1 golden is:

`3eaa82744580e8e080b1bbd48bfbc02a29096efc4e94a99fd53d68706a44820b`

Private validators then require the exact nonzero build hash and every header
field, 300 contiguous scheduled inputs, no game commands, no debug commands in
0–300, and exactly ordered unique checksum entries 1–300. Backward enumeration
through public `nearest_keyframe` has a 64-key cap and must account for the
reader's full `keyframe_count`, including rejecting future keys. The resulting
keys must be exactly the five listed above.

Every exact-key seek restores without simulating a tick. It validates registry,
entity/component shape, all Motion refs, coordinate bounds, tick and recorded
checksum before any final resimulation. Checked verify must then report exactly
300 simulated ticks, 300 checksums checked and no mismatch. Read-only seek first
runs full verification and then compares its target checksum with the recording
(or the pinned initial checksum at tick zero). The generic seek helper's
zero-build-ID Simulation stays private; it can never export a branch here.

The generic parser hides out-of-range debug keys and normalizes some duplicate
records. Only the closed byte allowlist justifies rejecting *all* encoded debug
commands, including ticks 0 and 301. The private validators are not a general
untrusted replay validator or an additional ORRP parser. Unknown bytes return
`UntrackedReplay`; locate the reviewed fixture/release rather than trusting a
runtime-supplied digest.

`System::run` returns `()`. A residual invariant failure after admission is fatal,
not a fallback profile or permission to continue. The tests deliberately avoid
panic/abort injection and replay-panic cases: incorrect admission uses ordinary
valid writer encodings and checks `Result` failures with zero system ticks.

## Golden and regeneration

`fixtures/checksums.txt` pins all 301 states, including tick zero. Existing games'
goldens are untouched. `fixtures/baseline.orrp` is the exact trusted recording.
Its writer is deterministic and always leaves the DebugCommand stream empty.

```sh
cargo test -p orr_asset_fixture
cargo clippy -p orr_asset_fixture --all-targets -- -D warnings
cargo fmt -p orr_asset_fixture -- --check
# Explicit candidate generation only; output directory must not exist:
cargo run -p orr_asset_fixture --example regenerate -- /tmp/new-fixture-candidate
```

The generator never updates its source-controlled allowlist or overwrites a
retained output directory. Compare/review candidate bytes and all checksums,
then explicitly replace this crate's fixture and digest if an intended change
is approved. A normal Cargo build performs no cook, filesystem search or network
fetch. Cooker files remain owned by the cooker workflow.

Dependencies flow fixture → asset/fp/ecs/sim/session plus host-side JSON/SHA.
The default feature graph has no cooker, audio, sample, graphics or device
dependency. The optional `audio` feature adds `orr_audio` and `orr_bridge`;
`orr_audio` default features stay disabled and do not open a native device.
Asset resolution itself is static binary search with no filesystem, lock or
allocation; this does **not** claim the entire engine tick/event recording is
allocation-free or supply a measured latency guarantee.


## Optional device-free presentation

`--features audio` enables `audio::FixtureAudio`. Construct it with
`FixtureAudio::embedded_offline(&fixture, AudioMode::Required)` or
`open_offline(&fixture, mode, ViewBundleBytes { manifest, objects })`. The caller
must first obtain the existing strict `PreparedFixture`; **auto/off never bypass
package or replay admission**. A later missing/corrupted presentation copy may
mute audio while that already admitted simulation continues. A wholly invalid
release package still fails `PreparedFixture::prepare` in every mode.

- Required: invalid view manifest/hash/object/header/budget or mixer failure is
  an error, before any audio update
- Auto: the same failure returns `AudioStatus::Muted { reason }`, no fallback
  clip or fake successful playback. The owner must expose this diagnostic
- Off: skips view validation/decode and mixer construction; rendering is silent
- Success: `OfflineReady` means real device-free Kira rendering, never a claim
  that a native stream opened or a person heard anything

Each declared PCM16 object is verified before allocation, converted with
`i16 / 32768.0` and copied to stereo, then owned by the clip bank. Only authored
`Impact { cue: 1 }` maps to `IMPACT_ID`; unknown cues are silent. Caller source
bytes can be dropped after construction. No loading, hash, allocation of sample
buffers, or cooking runs in tick/audio callbacks.

The presentation owner calls `Bridge::poll_view()` once, shares that immutable
batch with graphics and `audio.update(source_id, &update)`, and keeps the same
source ID for that bridge/session owner. Replacing the owner requires a fresh
ID. Forward the full ordered lifecycle/resync batch unchanged; snapshot epochs
are not source IDs. Existing EventAudio confirmation/cancellation/reset rules
and default 32 voices / 4096 history entries remain authoritative.

Limits: 16 view records, 128 KiB manifest input, 2 MiB actual/declared cooked
bytes, 48000 frames per clip and 4 MiB retained stereo samples across the bank.
All decoded counts are admitted before any stereo allocation. Current v1 schema
bounds are tighter for manifests (912 bytes) and cooked payloads (1536128 bytes),
so the generic byte-budget maximum is not itself a valid v1 package. Conversion
can transiently retain one additional clip-sized Vec (up to 384000 bytes) while
Clip creates its Arc backing. Mixer/allocator/input buffers add memory too; the
4 MiB retained-sample cap is not a whole-process peak-RSS guarantee.

```sh
cargo test --locked -p orr_asset_fixture --features audio
cargo clippy --locked -p orr_asset_fixture --features audio --all-targets -- -D warnings
# Informational baseline, no flaky timing threshold:
cargo test --locked --release -p orr_asset_fixture --features audio --lib measure_asset_tick_and_preload_baselines -- --ignored --nocapture --test-threads=1
```

Tests compare all 300 frame checksums and complete event payload/order between
real single-poll bridge runs with audio on/off and the admitted scalar runner.
They also cover finite/nonzero/bounded stereo, source-buffer lifetime, predicted
and late-verified dedupe, cancellation fade, same-key replacement, seek/branch,
pause/resume, resync, disconnect, source replacement, voice/history caps, exact
4 MiB decoded bank admission and the next frame's rejection. All audio tests
live in this crate. See `AUDIO_VALIDATION.md` for measured evidence and limits.
