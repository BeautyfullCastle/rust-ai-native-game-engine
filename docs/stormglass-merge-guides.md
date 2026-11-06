# Stormglass merge partner guides

Select an active unit on your local board. Compatible occupied cells show an
inset teal outline and a small plus badge, without requiring you to hover every
tile. Matching requires the same role and tier below `MAX_TIER`; support roles
can merge too. The selected source, empty/inactive cells and partner board are
never marked. At maximum tier there are no merge-partner badges; ordinary moves
to empty cells retain their existing behavior.

The shared native/readonly presenter painter derives guides from its current
Frame on every paint. A changed snapshot, rewind or completed merge removes stale
matches. Non-playing terminal phases hide guides. Vector units and custom unit
skins share the same overlay, clipped inside the local cell. Existing selected
outline, range diamond, hover hints, hit rectangles and command generation stay
unchanged.

Guides are advisory, not command admission. State or control readiness may change
before a click is consumed; the simulation still decides whether to move/merge.
Rendering never sends a command or performs a merge automatically. No simulation,
schema, economy, RNG or asset changes are introduced.

CPU acceptance covers local/remote, inactive/empty, role/tier/cap and duplicate
slot cases; a marked pair is merged through the actual game input path. Full
native/FrameView shape equivalence, custom-skin overlay retention, cell clipping
at three aspect ratios, unchanged Frame checksum/tick and changed same-tick
snapshots in one egui context are checked. This is geometry/simulation evidence,
not a raster image, native window/GPU/audio-device playtest or video. Existing
blocked window tools are not retried.

```powershell
cargo test --locked -p orr_sample --features coop-defense --lib merge_guide_tests:: -- --test-threads=1
cargo test --locked -p orr_sample --features coop-defense-controls --lib merge_guide_tests:: -- --test-threads=1
```
