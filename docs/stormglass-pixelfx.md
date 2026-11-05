# Native Stormglass pixelfx

`coop-defense-pixelfx` is an optional native presentation feature. It does not
change simulation components, commands, checksums, default builds, browser
presentation, or the registered editor presenter. Build and play with:

```sh
cargo run --locked --release -p orr_sample --features coop-defense-pixelfx --bin coop_defense
```

It embeds two actual effects from patch-gg/pixelfx commit
`a84159688af3ca37a4be05a0d422b94451b2e24e`: `impacts/hit-spark` (16 frames) and
`lightning/zap-hit` (28 frames), baked at 30 FPS with seed 4. Source metadata,
corrected PNG hashes and regression results are in
`crates/orr_sample/assets/stormglass-pixelfx/SOURCE.json`. The existing native
sprite packer admitted the PNG/SpriteDocument pairs before inclusion.

## Event and clock contract

The native `Session<DefenseGame, LocalInputSource>` delivers the real event
batch after each advance. Release shows a small spark at the actual emitter.
Hit shows a spark for roles 0/1 or an electrical burst for roles 2/3 at the
actual victim position, including splash hits. Death, disappearing enemies,
projectile expiry, snapshot differences and redraws do not invent impacts.
The existing damage-number feedback is independent.

Presentation time is derived only from the monotone session tick at 60 Hz.
Pause, focus loss, pre-start and inert redraws freeze effects. Duplicate
Predicted/Verified events do not restart them. Cancellation removes the keyed
effect; corrected payloads can replace canceled predictions. A real Restart
clears old effects and advances the generation. Loading a saved session clears
history and ignores old events at or before its restored baseline.

There are at most 64 active effects and 2048 retained event keys. Overflow
drops visual work, never gameplay. The bounded input batch admits twice the
game's public per-tick event maximum for local prediction and verification.
Old evicted keys cannot replay effects. Invalid producer identity/generation
is rejected transactionally. Like the native audio route, rollback across a
speculative Restart is not supported; this is a local-session feature, not a
general network rollback renderer.

## Pixels, assets and rights

Only compile-time PNG/JSON are loaded; no JavaScript, Python, network service,
model or arbitrary asset path runs in the game. Atlas decoding reuses
`orr_sprite`, texture uploads use straight RGBA via
`ColorImage::from_rgba_unmultiplied`, and filtering is nearest-neighbor. The
original pixelfx pivot is retained and rendering is clipped to the playfield.

The upstream sheet export multiplied RGB by alpha into an otherwise empty
atlas while retaining alpha. That caused dark fringes on later compositing.
These atlases copy each rendered frame's straight RGBA bytes directly. A
half-transparent red pixel stays `[255, 0, 0, 128]`, not `[128, 0, 0, 128]`.
Transparent alpha and seeded frame pixels are preserved. Additive light is
baked into an RGBA sprite; this is not a live additive-light compositor.

MIT LICENSE plus upstream NOTICE are present in both asset directories and
embedded in the optional native binary. `coop_defense --pixelfx-licenses`
prints them without opening a window. The excluded Patch Pixel gallery font
and stock character artwork are not shipped. Preserve these notices with
redistribution. PixelOE processing remains an offline, separately verified
step for approved character motion; it is not applied to these existing pixel
effects.

## Verification

`python3 tools/check_stormglass_pixelfx.py --evidence /absolute/new/directory`
checks exact focused test inventories and strict clippy for both VFX-only and
VFX/audio/run-save configurations, real App/Session playback, all existing
native defense regression with the combined features, native executable build, and
bundled-license output. The gate requires a clean committed tree and checks
that its commit, tree and clean status are unchanged after execution. Optional
builds do not replace
the mandatory Linux aggregate, exact-head remote focused CI or independent review.
The existing source-hidden audio-bank test is explicitly recorded as not executed
by this VFX gate; its separate fixture-driven audio acceptance remains unchanged.

CPU egui shape/texture tests prove data, UV/pivot, clipping and event wiring,
not rendered pixels or a hardware window. An actual software-rendered capture
can be produced through the existing trusted headless path:

```sh
coop_defense --headless --demo --ticks 300 --capture /absolute/new.png
```

The CLI reports the number of active/started/dropped/canceled effects. A
capture with no active effects is not evidence of their rendering. Software
GPU screenshots and a human visual review are reported separately; none of
these checks certify browser or physical hardware behavior.
