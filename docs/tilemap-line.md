# Tilemap Line brush

With the optional tilemap feature, open an admitted Arena tilemap project and
select **Line** in the tilemap tool row. Select a palette region or Erase, press
an in-grid cell, drag to a second cell, then release. The preview marks exactly
the cells that will change. A successful release is one tilemap Undo entry;
it does not change scene entities or scene history. Save tilemap separately.
The saved sidecar uses the existing tilemap document and runtime consumer.

The line is one cell wide, includes both endpoints, and uses integer Bresenham
rasterization. Endpoints are ordered by column then row before rasterization,
so dragging either direction paints the same cells, including exact slope ties.
This is not a filled rectangle or a supercover collision tool. Maximum length
is 64 cells on the existing bounded grid. Empty cells are distinct from region 0.

The selected region is captured at press. Invalid releases, Escape, loss of
focus/capture, tool changes, stale document/scene/source stamps, or unavailable
Edit mode cancel without committing. Existing combined actor/tile budgets and
complete candidate validation run before history changes. No-op or rejected
strokes preserve Redo. No implicit Save, new format, package change, or runtime
schema change is introduced.

Focused local validation (2026-11-04): 85 tilemap library tests passed with
`tilemap,project-create`, including four new line candidate tests and two
installed egui gesture tests. The installed path covers captured selection,
grouped Undo/Redo, cancellation/stale source, Save, fresh project admission,
and the runtime's five literal SpriteDraw placements. Strict editor library/test
Clippy and feature-off compilation also passed. Source review is independent.
These CPU/egui and runtime-data checks do not establish native-window, Windows,
hardware GPU, exported-player or hosted-CI acceptance. Existing mandatory gates
remain required separately.
