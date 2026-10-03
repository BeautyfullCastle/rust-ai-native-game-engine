# Focused validation: static fixture / release binding

Date: 2026-10-03 UTC. Local implementation base:
`271d20bf1c9a5a2378752ff552b9503ba9b9fe7c` (reviewed asset core/cooker tree).
Changes are restricted to `crates/orr_asset_fixture/**` and this package's
15-line Cargo.lock entry. Existing cooked assets, production `orr_sim`,
`orr_session/src/replay.rs`, and existing game goldens are unchanged.

## Environment and commands

Linux x86_64; rustc 1.99.0 (`b940084d7`, 2026-09-28), LLVM 23.1.1.
The focused Cargo lane used `CARGO_BUILD_JOBS=2` and the reusable target at
`/tmp/orrery-cooker-target`, not the nearly-full workspace filesystem.

Passed against the final source:

- `cargo fmt -p orr_asset_fixture -- --check`
- `cargo test --offline --locked -p orr_asset_fixture`: 18 unit tests and
  4 doctests, including 3 compile-fail API-boundary examples; no failures
- `cargo clippy --offline --locked -p orr_asset_fixture --all-targets -- -D warnings`
- Explicit candidate regeneration twice; both whole ORRP bytes and all 301
  checksum lines equal the checked-in baseline via byte comparison
- `cargo tree --offline -p orr_asset_fixture --edges normal`: no cooker,
  audio/sample/graphics/device dependency
- `git diff --check`

The initial clippy pass found two test idioms (`err().expect()` and a cloned
single-element slice). They were corrected; the final strict run passed.

Observed after focused validation: workspace free 90 MiB, /tmp free 4.2 GiB,
available memory approximately 7.5 GiB. These are observations, not peak-memory
or latency measurements. No broad cleanup, workspace/release build, graphics,
audio, or device test was run.

## Evidence and boundaries

- Full generated sim manifest SHA is pinned; bundle/object digests and canonical
  static-table re-encoding agree. Same GUID/new sim bytes fails before any tick
- Separate strict `orr.asset-release/1`, nonzero build pin and release-ID reuse
  rejection. A valid view-only update preserves gameplay replay identity
- Whole-file SHA allowlist is checked before parse. No public bypass exists
- Header/tick/input/debug/checksum/keyframe validators are exercised directly,
  independently of digest-only rejection. Normal ReplayWriter variants for
  SetField/Spawn/AddComponent and t0/t301 debug are refused with zero system ticks
- Exactly 300 ordered, unique checksum entries cover 1–300. Successful checked
  verify reports both ticks_simulated=300 and checksums_checked=300
- Descending public nearest-key traversal accounts for keyframe_count, rejects
  future/extra/missing keys and probes every exact key with zero resimulation
- Restored registry/ref/entity/component/range/tick/checksum checks are covered;
  invalid admitted data produces Result errors, without new panic/abort tests
- New baseline, rollback and selected seek boundaries match all relevant recorded
  checksums. x120=x180=7.5, x300=0; cue 1 fires at t1 and t181
- Seek exposes scalar state only. Generic helper zero-build identity stays private,
  and cannot export a branch/new recording

New fixture-only golden values:

- Initial: `7d81db837490e58b`
- Tick 120: `f26db18b42f69e25`
- Tick 180: `c174dce162c02552`
- Tick 300: `d4048d6f1fc625ff`
- Whole replay SHA-256:
  `3eaa82744580e8e080b1bbd48bfbc02a29096efc4e94a99fd53d68706a44820b`

A separate read-only review checked the complete new source/tests and fixture
identity. Its package-boundary finding (view root, cross-domain duplicate GUID,
PCM header checks) and identity-test coverage feedback were fixed and re-reviewed.
It reported no remaining P1/P2 finding, and independently checked the replay SHA
and 301-entry golden. It did not execute Cargo.

## Not established by this focused pass

No Windows, wasm, full workspace/release, audio playback, performance benchmark,
or remote CI result is claimed here. Existing default goldens were not rerun in
this lane; their source/fixtures and production behavior are unchanged. Remote
publication/CI/integration belong to the coordinator. Issue #35's opt-in audio
adapter and the parent asset epic remain separate work.
