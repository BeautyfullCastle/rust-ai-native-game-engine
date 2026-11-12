# One-shot terrain relief stamp

With the optional `orr_editor/terrain` feature, open a writable local terrain in
Edit mode, choose **Vertex**, and select the center with the vertex indices or
existing viewport selection. Expand **Stamp terrain relief**, set a positive
integer radius and signed decimal height delta, then **Preview terrain stamp**.
Review the changed-height count before **Apply terrain stamp**. This is one Undo
entry; Undo/Redo and the existing separate Save remain available.

This adds relief to the existing heights rather than flattening or smoothing:
for Manhattan grid distance `d <= r`, the fixed-point raw result is
`old + trunc_toward_zero(delta * (r-d) / r)`. The weighted delta is rounded before
addition. Computation uses wide integers and rejects any resulting overflow;
it does not clamp heights. The rim has zero weight. Terrain identity, grid,
origin, spacing, holes and all outside heights stay unchanged.

The whole diamond, including unchanged/rim vertices, must fit 4096 vertices and
lie inside the terrain. Radius 44 has 3961 vertices; radius 45 exceeds the limit.
There is no edge clipping. Any hole cell touching any vertex of the diamond,
including its zero-weight rim, rejects the entire operation. Unrelated holes
near a bounding-square corner do not block it. Existing render-admission limits
still apply atomically before document/history replacement.

Preview binds the current selected vertex/mode, exact parameter text, terrain
content/source, scene and load epoch. Field/selection/context changes revoke it;
failed Preview discards an older preview, and every Apply attempt consumes the
candidate. Apply rechecks the saved binding before editing. Identical-content
Undo may restore a valid binding; this is content binding, not a monotonic
revision token. Dirty terrain can be previewed. Play, stale attachment, previews,
read-only installed terrain and other existing authoring restrictions still
reject changes. No-op application preserves history/redo/dirty state. No file is
written until the existing Save action.

This is a single explicit stamp, not a continuous brush, erosion simulation,
collision/path clearance improvement, or new native-window interaction. Focused
acceptance uses actual egui controls and decoded terrain/model vertex data;
those checks are not physical-window or GPU-rendering evidence.
