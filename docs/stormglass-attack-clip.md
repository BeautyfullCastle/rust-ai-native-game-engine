# Rivet post-release sprite clip

Build `coop_defense` with `--features coop-defense-attack-clip`. This explicit
option includes the existing PixelOE idle skin and pixelfx event tracker. Default
vector art, idle-only builds and the prior procedural-recoil option remain
available. No external asset files or generator are needed at runtime.

A real retained `Release` event by the exact current Rivet actor selects a new
eight-frame `Once` clip: shot flash, arm recoil and recovery. Each frame lasts
40 ms; at session age 20 ticks (333 ms) the view returns to the existing idle
loop. A newer shot restarts at frame zero. There is no pre-release wind-up or
change to projectile timing, damage or simulation state. Duplicate verification,
cancellation, pause, actor identity, restart and save replacement reuse the
existing event-ledger lifecycle. The shared 64-effect budget can omit cosmetic
responses during large bursts. The old geometric recoil is suppressed while
this feature is enabled, so two motions are not applied to the same shot.

The new generated RGBA sheet is embedded unchanged at 1774x887. Explicit region
rectangles describe its 4x2 grid, whose cells differ by one pixel. Strict sprite
and PNG admission, nearest sampling, one fixed [222,362] pivot and 0.184 display
scale are used. No per-frame recentering or resampling occurs. Original idle
assets are unchanged. Generation prompts, notices and SHA-256 provenance are in
`crates/orr_sample/assets/stormglass-attack-clip/`.

The built-in image generator created the new sheet using the original idle
robot as an identity reference, then corrected its reactor to cyan. This is a
fixed forward-facing shooting pose; it does not aim toward individual targets.
Small pose redraw differences remain, and it is not a polished directional
animation set.

CPU checks cover exact asset hashes/alpha/uploads, all eight regions, fixed
pivot, tick sampling, one-shot expiry, invalid-role fallback and actual native
App event selection/pause/restart against unchanged Frame bytes. Existing
ledger and idle/recoil tests remain separate configurations. These checks do
not establish native-window/GPU or human-play acceptance. Any actual playtest
must include a short video and identify its rendering/input method.
