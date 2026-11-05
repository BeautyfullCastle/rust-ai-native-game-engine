# Optional PixelOE Rivet visual demo

Build the native player with `--features coop-defense-pixel-motion` and run
`coop_defense`. The default remains vector art. The feature can combine with
`coop-defense-pixelfx`, `coop-defense-audio` and `coop-defense-run-save`.
`coop_defense --pixel-motion-licenses` prints the embedded provenance and upstream
tool licenses without opening a window. No generator, external file or network
is needed at runtime.

## Contract

Only placed, active Rivet units receive this optional visual skin. Mortar,
Prism, Condenser, Haste and Miner retain vector art. The Rivet badge names the
role and tier; selection, range, placement and actual projectile/firing cues
remain in the shared view. Hitboxes, stats, damage, input and simulation state
are unchanged. Unprepared callers retain vector art.

The accepted 672×224 straight-RGBA atlas contains fourteen 96×112 frames in
row-major order, each 100 ms. The package, PNG, geometry, alpha and license bytes
are embedded unchanged, with source hashes in `assets/stormglass-pixel-motion/SOURCE.json`
under `crates/orr_sample`. Strict native SpriteDocument/PNG admission and nearest
texture sampling are reused. The fixed [48,85] pivot anchors the feet 20 logical
pixels below each cell center at 0.85 scale. Visible alpha fits the 78×68 board
cell and leaves room for the tier badge; padded transparent geometry is retained.
There is no per-frame recentering, rescaling, recoloring or pose editing.

The full 1.4-second cycle repeats only as a visual presentation. The accepted
SpriteDocument remains `Once`; presentation sampling applies the loop without
rewriting package metadata. The session-owned `DefenseRun.tick`, at 60 Hz and
relative to the current run, selects every frame. Paused and unfocused redraws
freeze that clock. Prestart and terminal screens use neutral frame 00; a real
restart resets the run clock, and loading a save restores its sample without
backlog, catch-up or replayed events. Frame 13 equals 00 exactly, preserving the
200 ms neutral hold across loop boundaries.

This is an idle/charge **visual demo**, not attack-synchronized animation.
`Release` is already a shot event and does not start this cycle. The existing
helmet, boot and glove redraw jitter, uneven easing and neutral hold remain.
The visible HUD explicitly labels the scope. Neither processing nor integration
claims production animation polish or independent generated-art rights review.
The preserved upstream tool licenses do not relicense the generated artwork.

## Acceptance

`python3 -B tools/check_stormglass_pixel_motion.py --self-test` validates the
false-green-resistant test parser and exact source asset inventory.
Run its `--evidence /ABSOLUTE/NEW/DIRECTORY` gate on a clean committed source to
execute the named CPU/App tests in skin-only and combined configurations,
strict Clippy, default/combined native regression, standalone notices and
feature-off/on native simulation parity. Evidence stays outside git and binds
to the source commit/tree and executable SHA. The focused remote workflow uses
the exact pushed head. Mandatory Linux aggregate and independent review remain
separate requirements.

Tests cover asset bytes, geometry, alpha uploads, all-frame visible bounds,
fixed pivot/UVs, the full loop and neutral timing, actual App pause/focus/start/
restart/load behavior, default vector fallback and identical simulation bytes.
These CPU/CLI checks are not a visual playtest. Every subsequent actual playtest
must include a short video; a software-rendered automatic replay must be labeled
as such, never as user-controlled or native-window footage. Hardware/display and
cross-platform validation require separate evidence.
