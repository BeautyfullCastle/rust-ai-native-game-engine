# Orient an imported model

In local Yard3D or Room Edit mode, open model bindings and choose the model.
Use **Rotate X -90/+90**, **Rotate Y -90/+90**, or **Rotate Z -90/+90** to turn
it by a quarter turn around its current local axis. Positive turns follow the
right-hand rule: from identity, Y +90 turns forward +Z toward +X. Successive
turns compose in button order using model-local axes.

These controls change only the rotation draft. **Reset model rotation** restores
identity, including recovery from an invalid manually entered quaternion. Offset,
scale and animation clip/playback choices remain intact. Choose **Assign static
model** or **Assign animated model**, then **Save model bindings** to apply the
orientation. Existing model Undo/Redo and Save/reopen retain the saved transform.
The runtime composes that local orientation with the body's orientation.

The quaternion inputs remain available. Quarter turns reject invalid/nonfinite
or non-normalized input, bad axis/direction and unavailable Edit authority
without modifying the draft. Play, previews, remote or mismatched sources block
both actions. Existing Assign validation still checks the complete transform.

Rotation edits are presentation-only: they do not move or rotate the physics
body/collider, change geometry or write files themselves. Fitting a model resets
local orientation; orient it after fitting when needed. A rotated model need not
remain inside its old collider fit.

Acceptance covers right-handed axes, local composition, repeated normalization,
invalid inputs, actual CPU widgets, static and animated Save/reopen/host poses,
Play guards and saved orientation in a source-hidden deployed Room runtime.
Native window/GPU appearance is separate unverified coverage.
