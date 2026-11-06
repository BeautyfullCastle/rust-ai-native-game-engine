# Stormglass core approach warning

During an active game, enemies on the final route leg (progress 1600 through 1999)
receive a static red halo and an exclamation badge. The shared core caption reads
`CORE APPROACH` while at least one such enemy remains. The warning overlays the
core shell so a nearly leaking enemy remains marked. Existing slow rings and
health bars remain visible.

This is a route landmark, not an estimate of distance, arrival time, or damage.
The two lanes have different geometry; enemy families and slow effects also
change travel speed. Progress 2000 is the simulation's leak boundary. Inactive
enemies, completed games, and progress outside the final leg do not warn.

The shared native/readonly painter derives every mark from the current Frame.
Paused redraws are static; replacement snapshots and rewinds recompute the marks.
The feature does not change targeting, movement, input, health, economy, or RNG.
Parent painter clipping applies at every screen size.

Focused tests cover the boundary, real movement and leakage, both lanes and boss
sizes at three aspect ratios, terminal phases, immutable and equivalent renderer
paths, same-tick replacement, redraw, and overlay ordering. These are CPU shape
and simulation checks. Native-window appearance, GPU output, audio devices, and
playtest video remain unverified.
