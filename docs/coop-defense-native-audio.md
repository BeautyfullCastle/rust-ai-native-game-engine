# Native Stormglass authored audio

The optional Linux `coop-defense-audio` player consumes the existing fixed
CoopDefense soundbank. It uses real Session events and the existing Kira mixer.
It does not change combat, save journals, replay identity or the web audio policy.

## Use

Create a bank with the existing [Rust audio authoring CLI](rust-coop-audio-bank.md).
Supply your own permitted PCM/WAV inputs and an explicit license. Existing
recovered sample bytes are not evidence of redistribution rights.

Build native output explicitly:

```sh
cargo build --release -p orr_sample \
  --features coop-defense-audio,audio-native,coop-defense-run-save \
  --bin coop_defense
./target/release/coop_defense --scene /absolute/game.scene.yaml \
  --audio-bank /absolute/published-bank --audio auto
```

`--audio-bank` selects the directory containing `bank.json` and ten fixed PCM
filenames. Linux admission uses bounded no-follow reads and a held directory;
use trusted, stable ancestor directories. No downloads, arbitrary manifest paths,
Node, browser packaging or codec execution occur in the native consumer.
An explicitly supplied invalid bank is an error, even with audio off.

`--audio off` never opens a device. `auto` reports unavailable output and continues
silently. `required` fails rather than silently substituting an offline mixer;
it requires a bank and a build with native output. Headless required output is
not supported. A successful CPAL stream open is not proof that a listener heard it.

The game starts muted. Use the top-bar mute/unmute control and minus/plus gain
controls. Gain is bounded and presentation-only; settings are not saved. Mute,
pause and focus loss stop transients, with a short bounded fade. Focus return
still requires explicit Continue, and unmuting does not replay missed sounds.

## Content and event contract

The shared admission retains the existing version-one bank contract: ten fixed
cue names and asset IDs, 48 kHz mono PCM16, 1–48000 frames per clip, exact byte
counts and SHA-256, peak at most 8192, 128 KiB manifest, 2 MiB cooked and 4 MiB
decoded budgets. Unknown JSON fields and cue order retain their existing meaning.
All objects are admitted before any output is created. No resampling, runtime
asset editing or new bank schema is added.

Only emitted Session summon, merge, enemy death, attack release and impact events
map to clips. Release roles 0/1/2/3 select light/heavy/heavy/magic; impact roles 1/2
select heavy and 0/3 light. Support/mining roles and UI confirm/reject remain
unmapped. Rejected purchases, movement, upgrade and invest intentions do not
invent sounds.

Session event keys, order, prediction, verification and cancellation are retained.
A predicted event later verified plays once. Restart creates a silent generation
boundary once; its later verification does not reset output again. This is the
local player's monotone restart path, not generic speculative-restart rollback.

Detached Run Save reconstruction has no audio owner. A successful atomic restore
replaces the binding and establishes a silent baseline at the restored tick,
while retaining mute/gain and the restored paused state. Failed or cancelled
loading does not replace the binding. Only new accepted events after explicit
Continue can sound. The chosen audio bank is independent of the gameplay save;
a save does not embed or recover that bank.

The native mixer is constructed at zero gain before any event can play. The
owner caps effective gain by its bounded voice count, including fading voices.
Later gain changes may interpolate over one internal mixer block; they remain
within that bound. Optional output failure disables audio without changing gameplay;
required-mode runtime failure remains an error, including while paused or in a
save dialog. Device reopen/shutdown behavior retains the pinned backend's limits.

## Acceptance boundary

Focused acceptance uses newly generated original licensed PCM through the real
authoring CLI, relocates the published bank and hides original inputs, then
checks native admission and actual Kira mixed PCM. CPU App/egui/Session tests
cover mapping, deduplication, cancellation, gain/mute, pause/focus, restart,
restore and audio-on/off gameplay equality. Device errors use an explicit injected
seam; native-output wiring is compiled, not opened by these tests.

Independent source review, exact-head required CI and downloaded-artifact
re-execution precede development integration. Offline PCM is not physical-device
listening, native-window, browser, GPU, Windows or full-workspace acceptance.

The reproducible gate is `python3 tools/check_stormglass_native_audio.py` with
`EVIDENCE` set to a fresh directory outside the checkout and genuine ALSA
development files installed for native compilation. It verifies exact test
inventories, strict production checks, fixture and producer hashes, and the
compiled native binary in headless audio-off mode. The required workflow retains
its artifacts even on failure.
