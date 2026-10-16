# Arena audio vertical slice

`orr_audio` is a presentation-only Kira 0.12.5 mixer adapter. Its default
build has **no device, decoder, realtime-scheduling or D-Bus dependency**.
`orr_sample` opts in through `audio`; `audio-native` adds CPAL for desktop
playback. The optional audio feature requires Rust 1.86 or newer (Kira's
`triple_buffer` dependency); the simulation crates keep their existing MSRV.
Simulation state, replay formats and golden checksums are unchanged. Since the
workspace includes every crate, aggregate `cargo test --workspace` also includes
`orr_audio` and therefore requires at least Rust 1.86 (or any higher minimum
already required by other workspace members).

## Run

```sh
# Default build remains device-free. This reports unavailable; gameplay continues.
cargo run -p orr_sample --bin arena -- --audio auto
# Explicit opt-out never attempts device initialization.
cargo run -p orr_sample --bin arena -- --audio off
# Linux requires the distribution's ALSA development package (libasound2-dev).
cargo run -p orr_sample --features audio-native --bin arena -- --audio required
```

Modes are `off`, `auto` (default), and `required`. Auto prints the reason if
native output was not compiled, stream opening fails, or a runtime stream error
is reported. Required fails instead of silently continuing muted. Required audio
is rejected in the existing headless relay-bot path, which has no audio owner.
The window loop maps Arena `Hit` events to one original, quiet 150 ms procedural
impact, generated once and shared between voices. No external sound asset or
codec is used. Native initialization logs **stream opened**, not proof of audible
speaker output. Reported stream errors are polled each frame; this slice disables
output on an error rather than promising automatic recovery. Kira's pinned CPAL backend
has an upstream limit: a failed automatic device reopen can panic its background
worker before forwarding an error, so polling is not a complete health monitor.
Its shutdown worker polls at 500 ms; dropping the owner queues immediate voice
stops and requests backend shutdown, but is not a synchronous device-close fence.

## Event ownership and bounds

- Consume `Bridge::poll_view()` once and pass its full ordered `ViewUpdate` to
  `EventAudio::update` before moving the events. Use a different source ID when
  replacing the bridge owner. Source IDs belong to the presentation caller.
- Predicted starts a one-shot. Verified confirms it without starting another,
  even if the original voice finished. Verified-only events also start once.
  Completion tombstones remain after handles are reaped.
- Canceled fades the owned voice over 15 ms by default. A subsequent Predicted
  with the same key is a replacement occurrence. A stale Verified after cancel
  is suppressed. Already-played samples cannot be undone by rollback.
- Seek, branch, source replacement, session start, resync and disconnect fade
  old voices. Seek/branch/resync also retire notifications through their baseline
  tick, so old confirmations cannot reconstruct missed one-shots. Disconnect
  suppresses all remaining events until a new source/session. Pause stops
  transients but keeps tombstones; resume does not replay them.
- Normal snapshot epochs do **not** rebind the event batch: a snapshot may be
  newer than its event tail. Ordered lifecycle notifications define changes.
- Default bounds are 32 voices (including fading/draining voices), 4096 event
  history entries, and 10 seconds per clip. At voice capacity, newest sounds are
  dropped and tombstoned. At history capacity, the oldest whole tick is retired
  behind a monotone stale floor and its voices fade. This deliberately favors
  silence under overload over duplicate playback. Statistics expose starts,
  confirmations, cancellations, completions, duplicates, drops and resets.
- The audio manager owns source sample memory and voice handles. Dropping it
  requests voice stops and backend shutdown. There is no render-to-simulation path,
  history sample buffer, asset database, spatialization, streaming music or browser audio integration.

## Verification

```sh
cargo test -p orr_audio
cargo test -p orr_sample --features audio --lib arena_audio
cargo clippy -p orr_audio --all-targets -- -D warnings
cargo check -p orr_sample --features audio-native --all-targets
```

The offline backend drives Kira's actual `Renderer` into caller-owned interleaved
stereo PCM at 48 kHz. Tests assert finite/nonzero/bounded samples, actual
cancellation silence after the fade, exactly-one onset including late verification,
owned-source lifetime, reset/seek/resync/pause behavior, and bounded saturation.
The sample test feeds actual Arena bridge Hit events to the mixer and compares
the event stream and every predicted-frame checksum with an audio-free run.
Linux/Windows CI compile the optional native path; these checks do not prove hardware playback. A physical-device
listening run remains required for that claim.

Official dependency references: [Kira 0.12.5](https://docs.rs/kira/0.12.5/kira/),
[backend API](https://docs.rs/kira/0.12.5/kira/backend/trait.Backend.html),
[CPAL](https://github.com/RustAudio/cpal). The pinned crate manifests were checked:
Kira default features are disabled and CPAL's own default feature list is empty.


## Opt-in cooked asset fixture

`orr_asset_fixture --features audio` is a separate device-free vertical slice;
it does not replace Arena's clips or change its default audio contract. Its
`FixtureAudio` requires an already admitted `PreparedFixture`, verifies the
release's view SHA and every PCM16 object's length/hash/header, and owns stereo
clips before event consumption. Only fixture cue 1 maps to its declared impact
asset. Pass the complete result of one `Bridge::poll_view()` once with a stable
source ID, including lifecycle/resync; use a new ID for a replacement owner.

Required failures return an error. Auto failures return an explicit muted reason;
off skips preload and mixer construction. These policies apply only to the
presentation copy after strict fixture admission; missing/corrupt release
packages still fail the unchanged preparation gate. The fixture currently opens
no native output device in any mode and makes no physical-playback claim.

```sh
cargo test --locked -p orr_asset_fixture --features audio
cargo clippy --locked -p orr_asset_fixture --features audio --all-targets -- -D warnings
```

Bounds, public usage, source lifetime and measured validation are documented in
[`orr_asset_fixture`](../crates/orr_asset_fixture/README.md#optional-device-free-presentation)
and [its audio validation record](../crates/orr_asset_fixture/AUDIO_VALIDATION.md).
