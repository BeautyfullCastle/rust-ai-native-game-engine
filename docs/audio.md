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

## Spatial sound

A clip placed in the world with `Clip::at(position, near, far)` is heard from
the listener set by `EventAudio::set_listener(Listener { position, right, velocity })`.

- Volume: full within `near` meters, then inverse distance (twice the distance
  is half the amplitude, -6 dB), faded to silence over the last fifth before `far`.
- Placement: the sound pans toward the side it is on, from the angle between the
  listener's `right` and the direction to the sound.
- A playing sound follows the listener: `set_listener` retunes the volume and
  panning of every playing spatial voice over 20 ms, so movement does not click.
- Doppler: a source that moves (`Clip::moving(velocity)`) or a listener that
  moves (`Listener::velocity`, meters per second) shifts the pitch by the
  playback rate `(c + v_listener) / (c + v_source)`, with both speeds taken
  along the line from the listener toward the source and `c` = 343 m/s
  (`SPEED_OF_SOUND`). Each speed counts as at most 0.9 c and the rate stays
  within 1/2..=2 (`MAX_DOPPLER`). Motion across the line of sight changes nothing.
  `AudioConfig::doppler` scales both speeds: 1 is physical (the default), 0 turns
  the effect off, values up to 10 exaggerate it.
- A moving source: `EventAudio::move_emitter(key, position, velocity)` moves the
  playing voice of an event, and its volume, panning and pitch glide to the new
  values over 20 ms. Call it each frame for a car or a projectile passing by.
- `Emitter::heard_by` returns the volume in decibels and the panning, and
  `Emitter::doppler` the playback rate, for a caller that wants to show or log them.
- Not modeled: occlusion by walls, elevation (a sound above and one below
  sound the same), and the travel time of sound (a far sound starts at once). The
  rate changes the clip's length too: a clip closing in at a fifth of the speed of
  sound plays 20 percent shorter.

## Reverb

One reverb bus is shared by every voice (a send/return setup): each voice has a
send level, and the bus returns only the reverberated sound, so the dry sound is
not changed. The reverb is Freeverb-style: 8 damped comb filters in parallel per
channel into 4 allpass filters in series, the right channel tuned 23 samples
longer for stereo width, and an optional pre-delay. All buffers are allocated
when the bus is created; the audio thread allocates nothing.

- `ReverbParams { room_size, damping, pre_delay, wet }`: room size 0..1 (tail
  length), damping 0..1 (how fast the highs die), pre-delay in seconds up to
  `MAX_PRE_DELAY` (0.1), and `wet` 0..1, the send level of a placed sound.
  Presets: `OUTDOOR` (wet 0, no reverb), `SMALL_ROOM`, `HALL`, `CAVE`.
- A zone is `ReverbZone::sphere(center, radius, params)` or
  `ReverbZone::cuboid(center, half_extents, params)`, with `.fade(meters)`
  (default 2). Its weight is 1 inside and falls linearly to 0 over `fade` meters
  outside. The listener position picks the parameters: overlapping zones are
  averaged by weight, and `set_ambient_reverb` (default `OUTDOOR`) takes the
  share the zones leave to 1, so leaving every zone fades the reverb out.
  `reverb_at(position)` returns the parameters heard at a position.
- Placed sounds (`Clip::at`) are sent as much as `wet` says. A clip that is not
  placed has a send of 0 unless it asks: `Clip::reverb(send)` (0..1) sends any
  clip, placed or not, at that level; the zones then only set the room.
- `set_listener`, `add_reverb_zone`, `remove_reverb_zone`, `clear_reverb_zones`
  and `set_ambient_reverb` move the send of every playing zone-driven voice over
  20 ms, and the bus's room size, damping and pre-delay glide with a 30 ms time
  constant, so crossing a zone edge does not click.
- Cost: nothing exists until a zone, a non-silent ambient reverb or a clip send
  asks for it. With none of them the output is bit-identical to a build without
  reverb. Once the bus exists it runs for the rest of the session. A voice gets
  its own mixer track (and a path to the bus) only if reverb was in use when it
  started; a sound started earlier stays dry. At most `MAX_REVERB_ZONES` (64)
  zones.
- Limits: one bus, so one room at a time (the listener's); sounds are not
  reverberated by the room they are in, only by the room the listener is in; the
  send does not grow with distance; no early reflections from geometry.

```rust
use orr_audio::{AudioConfig, Clip, Listener, OfflineAudio, ReverbParams, ReverbZone};

let mut audio = OfflineAudio::new_offline(AudioConfig::default(), 48_000)?;
// A hall around the origin that fades out over 6 m, a cave elsewhere.
audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 20.0, ReverbParams::HALL).fade(6.0))?;
audio.add_reverb_zone(ReverbZone::cuboid([100.0, 0.0, 0.0], [15.0, 6.0, 15.0], ReverbParams::CAVE))?;
// Placed sounds get the reverb of the zone the listener stands in.
let step = footstep.at([2.0, 0.0, -3.0], 1.0, 40.0);
// A UI sound is not placed: it asks for a send of its own.
let chime = chime.reverb(0.3);
audio.set_listener(Listener { position: [100.0, 0.0, 0.0], ..Listener::default() });
```

`cargo run --release -p orr_audio --example reverb -- <folder>` renders a click
and tone sequence dry and in the hall to `reverb_dry.wav` and `reverb_hall.wav`.

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
  history sample buffer, asset database, streaming music or browser audio integration.

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
