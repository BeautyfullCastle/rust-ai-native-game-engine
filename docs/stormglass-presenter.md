# Registered Stormglass vector presentation

The static starter's `stormglass` feature opts into
`orr_sample/coop-defense-presentation`. This small presentation feature exposes
`defense_view` with egui and the existing Coop Defense types; it does not enable
`defense_app`, `defense_ui`, `project`, or `game-ui`. The existing native
`coop-defense` feature includes it, preserving `paint(&Frame)` callers.

`paint_view(FrameView)` is the shared read-only painter: two boards, the exact
simulation path, role/tier silhouettes, active enemies and HP bars, actual
projectiles, selection range, and reactor. Component iteration preserves the
previous entity-index order (units additionally sort by board/cell), rather than
using swap-removal-sensitive dense order. Rendering never changes the frame.

The registered project presenter keeps its HUD, source/target controls, and
semantic actions. Its remaining viewport is an aspect-fit, clipped canvas.
Clicking a visible cell on the left board changes only the target selection;
it does not spend gold or move a unit. Source cell remains explicit. Right-board
and clipped-out cells are not selectable. Editing the target control updates the
highlight. Reset and a changed run generation clear local source/target/selection.
The raw enemy list remains available as a collapsed diagnostic section.

This reuses existing vector artwork. It is not a new simulation, polished sprite
atlas, damage-number/event FX system, audio integration, or proof of platform or
hardware GPU support. Focused verification covers layout/hit clipping, identical
wrapper/read-only shapes, frame immutability, presenter reset, and the existing
actual CLI/Stormglass WGPU play-loop capture. Captures are external evidence,
not committed assets; opening those images remains part of visual review.
