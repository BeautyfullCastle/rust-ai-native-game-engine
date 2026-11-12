# Aim the game camera target at the mesh agent

The optional mesh camera authoring panel has an **Aim target at mesh agent**
button. It copies the current coherent local mesh agent position into the three
camera target fields. This is useful when positioning a shot around the authored
agent without transcribing its coordinates.

Turn **Follow mesh agent** off, then press the button in **Mesh game camera**.
The yaw, pitch, distance, projection, field of view, near/far planes and follow
offset fields stay unchanged. The live viewport, applied camera document,
history and files also remain unchanged. Use **Apply game camera**, then the
existing Save workflow. Undo/Redo operate through that existing applied-camera
history, not through a hidden scene edit.

This is **Aim**, not Frame/Fit: it does not inspect model bounds, choose distance,
ensure the entire agent is visible, or remove occlusion. It uses the same
fixed-point-to-view-position conversion as the existing follow-camera consumer.
The target must pass the existing camera target admission. Other invalid draft
fields are not silently repaired; Apply still validates the complete camera.

Follow mode must be disabled explicitly so aiming never reinterprets the follow
offset or silently toggles tracking. The existing local Edit, coherent snapshot,
matching scene/graph, stale-sidecar, gesture, proposal and preview guards apply.
The button reads the current agent at click time, not a cached previous position.
It is processed after ordinary form edits. A frame that requests projection
matching disables Aim so a second action cannot hide a conversion error. The
aiming result is not cleared by earlier same-frame field housekeeping.

Core camera format, renderer, simulation and saved sidecar schema are unchanged.
Focused acceptance exercises the actual button, unchanged pose/settings before
Apply, Apply/Undo/Redo/Save/Reopen, blocked contexts, and same-frame projection
interaction. CPU/editor checks do not certify GPU rendering or shot composition.
