# Mesh hazard authoring

Build the editor with the opt-in `mesh-hazards` feature and explicitly select
`--game mesh-hazards-v1` for a local scene with an admitted mesh graph. The
`MeshHazardsV1` registry is separate from legacy `MeshNavigation`: legacy scenes
and the default editor do not acquire hazard singletons or gameplay behavior.

The **Mesh hazards** panel edits `MeshHazards` only:

- Enable hazards and choose 1–8 spheres in authored order
- Enter center X/Y/Z and radius as exact fixed-point decimals, without float conversion
- Centers must be within ±256 units and radii positive and at most 256 units
- Spheres may overlap a route without their centers lying on the mesh surface
- Apply validates against the current admitted graph through the remote hazard
  patch builder, then submits one host singleton edit
- Apply clears the draft only after exact authoritative configuration readback
  from the same scene-load epoch and graph
- Disable and Apply writes the canonical all-zero configuration; unused sphere
  slots are always zero
- Save the scene explicitly after Apply; Undo/Redo use the existing host history
- Reload discards draft fields and reads the current authoritative configuration

Runtime `MeshHazardRun` loss/index/tick is displayed read-only. It is never an
editable panel field. Contacts use the host's swept-path rules, with authored
order defining hazard indices; the panel does not implement collision logic.

Dirty fields block Play and workspace changes. Scene loads (including reloads of
the same path), graph changes and other edits invalidate a draft. Apply checks
identity again at action time, including checksum, even before the next UI sync.
Play, preview, pending proposals, gestures, disconnected or non-local hosts,
and missing/unadmitted snapshots cannot apply or reload authoring fields.

This slice does not add hazard workspace creation, hazard asset editing,
viewport picking or standalone hazard export. Use an explicitly selected local
hazard scene; unsupported workflows remain unavailable rather than silently
falling back to the legacy profile.

Headless tests cover decimal/count validation, bounded geometry, canonical
zero disable, exact readback, Undo/Redo, checksum/epoch/graph stale-draft guards,
Play rejection, legacy-profile isolation, and actual egui Apply/Reload actions.
