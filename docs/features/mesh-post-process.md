# Saved MeshNavigation HDR and bloom

MeshNavigation games rendered by Yard3D can persist HDR/bloom in their optional
mesh-lighting sidecar. In **Mesh game lighting**, edit **Saved HDR post-processing**
and bloom controls, then **Apply game lighting** and **Save game lighting**. Undo,
redo, reopen, stale-writer rejection and Play/export dirty guards use the existing
lighting document history. Lighting presets preserve the target's post-process
policy. This does not change simulation state.

Legacy schema 1 has no post-process field and defaults to off; its encoded form
is unchanged. Authored settings use schema 2, including explicitly disabled ones.
Unknown/duplicate fields, explicit null, arrays, nonfinite and out-of-range values
are rejected. Renderer bounds apply even while disabled. Exposure and tonemapping
remain lighting controls; they are not duplicated by HDR settings.

The Mesh Player consumes the saved policy through the same renderer selector.
Export and bundle reopening require a compatible PE runtime advertising the
`.orrpost` / `ORR_MESH_POST_PROCESS_V1` capability for schema 2. A marker in an
executable overlay does not qualify. Existing PE-only export admission is unchanged.

Unsupported HDR, invalid post-process settings or the HDR color-budget limit
falls back to HDR off for that scene/load/policy. The warning remains visible;
saved authoring settings are never rewritten. Unrelated model/render errors are
not mislabeled as HDR rejection. A changed scene/load/policy retries admission.

This scope does not persist effects for Room, MeshHazards, other Yard3D games or
2D games. Tests cover strict wire data, disk roundtrips/conflicts, panel history,
headless egui controls, frame-policy selection and export admission. These are
not hardware/GPU, Windows or actual Player execution certification.
