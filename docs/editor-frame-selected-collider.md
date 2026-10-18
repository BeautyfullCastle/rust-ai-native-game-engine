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

The yellow selection wire follows the admitted collider surface, including its
rotation. Thin and tall boxes therefore show their actual narrow footprint and
fit inside the same framing margin. Sphere and capsule curves use bounded
32-segment circles. Imported model bounds remain excluded.

Historical QA: the original framing screenshot used the legacy oversized yellow
cube (1.08 times the largest collider half-extent on every axis). That outline
could extend offscreen although the collider fit correctly. The original QA
images are retained unchanged; the corrected wire is verified in fresh captures.

Framing does not edit the authored camera document, simulation Frame, history,
selection, scene or model files. In a Room, use the separate explicit **Stage
viewport camera** and Apply/Save workflow if you want to author this framing.

The software-Vulkan UI test selects an offscreen tall wall from the hierarchy,
checks all eight corners fit with margins after both the button and focused F,
and verifies session-only behavior. CPU tests cover perspective/orthographic
fits and unsupported/oversized refusal. Synthetic input is not a claim of
physical OS or hardware-GPU acceptance.

A separate Yard3D software-Vulkan case selects a rotated box and a capsule through
the hierarchy, frames each, and compares rendered yellow pixels with projected
collider curves while checking scene bytes, Frame, history and selection remain
unchanged by framing.
