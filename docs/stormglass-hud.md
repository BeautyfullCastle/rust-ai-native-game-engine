# Stormglass compact HUD

The project presenter keeps wave, reactor health, both players' gold, kills,
leaks and a readable game phase above the board. Source/Target cell selectors
and Summon, Move / merge, Upgrade, Invest and Restart stay visible. When input
is unavailable, the Play / Take control guidance remains visible.

**Details** opens setup help, target selection guidance, receipt semantics and
the authoritative enemy list in a scroll area capped at 100 logical pixels.
Closing it returns space to the board. It does not send a gameplay command or
change the selected cells. The editor's Take/Release control, command status,
errors and cleanup warnings are unchanged and never moved into Details.
A consumed command is still not proof of a successful purchase or move.

Focused acceptance covers repeated disclosure toggles, selection and readonly
snapshot preservation, control visibility, and actual 960×720 offscreen
framebuffers. The capture test takes `ORR_STORMGLASS_HUD_BASELINE`, a verified
previous focused Stormglass edit-state PNG at the same resolution and scene,
and requires more than 10% growth in rendered board tile area. The external
runner pins that baseline and its prior evidence; it is not a synthetic image.
It also checks scene/history/tick/checksum stability and records the comparison.
Software-WGPU captures do not certify native windows, physical GPUs or combat.
