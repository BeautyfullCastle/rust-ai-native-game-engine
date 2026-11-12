# Stormglass board picking

Choose **Pick source** or **Pick target**, then click a cell on your left board.
Source has an inner orange outline; target keeps the outer teal outline. Both
remain distinguishable when they name the same cell. Numeric cell inputs remain
available. Clicking cells or changing picking mode does not submit a command.
Press **Move / merge** to request the existing action with those two cell values.
The authoritative game decides whether an empty move or compatible merge is valid;
a consumption receipt alone is not proof of success.

Clicks on the other board, the background, or clipped areas are ignored. A new
run generation/reset clears both outlines and returns picking mode to target.
Summon still places a unit in the first empty cell, not the selected target.
The generic Take/Release control and command error behavior is unchanged.

Acceptance is scoped to selection/payload and reset tests plus one actual
registered-editor pointer-picked move and software-WGPU capture. This does not
certify native window input, hardware rendering, or all combat behavior.
