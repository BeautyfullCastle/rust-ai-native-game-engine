# Frame a selected Room or Yard3D collider

Select an entity in the hierarchy, then choose **Frame selected collider** above
the viewport. Alternatively, give the viewport focus with a right or middle click
and press **F**. The camera moves its session target and distance to contain the
selected collider with a margin. Orbit angles and field of view stay unchanged.

This uses conservative **collider bounds**, not imported model bounds. Room and
plain Yard3D are supported. Missing, unsupported or oversized colliders are
refused without changing the camera. Play, dialogs, text entry, placement/drag
gestures and active content jobs disable the command. F has no effect without
viewport focus.

Visual limitation: framing fits the actual admitted collider. The existing yellow
selection outline is an oversized cube whose half-extent on every axis is 1.08
times the collider's largest half-extent. For thin or tall shapes, that cube can
extend beyond the viewport even when the actual collider fits with a margin.
The outline is not the collider boundary; imported model bounds are also excluded
from this framing command.

Framing does not edit the authored camera document, simulation Frame, history,
selection, scene or model files. In a Room, use the separate explicit **Stage
viewport camera** and Apply/Save workflow if you want to author this framing.

The software-Vulkan UI test selects an offscreen tall wall from the hierarchy,
checks all eight corners fit with margins after both the button and focused F,
and verifies session-only behavior. CPU tests cover perspective/orthographic
fits and unsupported/oversized refusal. Synthetic input is not a claim of
physical OS or hardware-GPU acceptance.
