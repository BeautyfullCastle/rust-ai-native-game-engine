# Stormglass action cost hints

The compact HUD shows Summon, Upgrade and Invest costs from the displayed player-zero
snapshot. When gold is short, it shows the missing amount. Upgrade and Invest show
“maximum reached” at their respective caps. Costs and caps come from the existing
shared game constants and functions; no separate UI economy rules are introduced.

Hints are advisory, not action admission. An apparently affordable action can still
be rejected because the board is full, the run is over, or state changes before
execution. Buttons keep their existing control-readiness rules and command payloads.
A consumed command is not necessarily a successful purchase. Inspect authoritative
gold, levels and units to confirm the result. Summon still uses the first empty cell,
not the selected target. Detailed explanation is available under Details and on hover.

Acceptance covers shared-cost boundaries, shortfall/cap rendering, unchanged command
payloads even when hints show a shortage or cap, and an actual registered-editor
software-WGPU capture with unchanged scene/history. It does not certify native GPU
rendering or repeat combat acceptance. Move/merge eligibility guidance is outside
this change.
