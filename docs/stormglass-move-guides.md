# Stormglass empty-cell move guides

Select an active unit on your local board. Empty local slots show bronze corner
brackets and **MOVE**, distinct from occupied merge partners' teal outline/plus.
The selected source, occupied cells, partner board and unavailable slots are not
move destinations. Maximum-tier units still move normally; their lack of merge
partners does not disable movement.

The shared native/FrameView painter recomputes destinations from the displayed
Frame on every paint, with no timer or cached list. A newly occupied cell or stale
source immediately removes the label, including same-tick snapshot replacement
and rewind. Terminal/non-playing phases hide guides. Overlays remain above unit
skins and inside both the target cell and caller clip.

These are advisory hints, not admission or automatic commands. Existing clicks,
hover hints, cell hit rectangles, sequence/control readiness and actual game
move/merge rules remain unchanged. The simulation may reject a command if the
state changes before consumption. Rendering never moves a unit itself and does
not alter Frame, RNG, economy, schema or assets.

CPU acceptance checks empty/local/occupied/source/missing/duplicate slot cases,
and an actual maximum-tier move preserving actor ID, role and tier without a
merge. Native/readonly shape parity, nine expected destination labels, responsive
geometry, opaque skin overlay order, partial/hidden clips and same-context
snapshot/terminal changes are covered, with stable source tick/checksum. Existing
merge guides, view and HUD command tests remain covered. These are not a raster
capture or native window/GPU/audio-device playtest/video; blocked window tools
are not retried.

```powershell
cargo test --locked -p orr_sample --features coop-defense --lib move_guide_tests:: -- --test-threads=1
cargo test --locked -p orr_sample --features coop-defense-controls --lib move_guide_tests:: -- --test-threads=1
```
