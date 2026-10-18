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

## Create a new hazard game

With `mesh-hazards` enabled, **File → New hazard game…** is available from an
idle local Physics, legacy MeshNavigation, or MeshHazardsV1 editor. Enter a new
absolute `*.scene.yaml` path in an existing directory, floor dimensions,
collectible count and fixed-point distance per tick, then choose **Create hazard
game**. The shared mission generator builds the walkable floor and collect/exit
mission, and the new file opens explicitly as `MeshHazardsV1`.

Hazards start disabled with a canonical all-zero configuration. The optional
**Include starter hazard sphere** checkbox adds a radius-0.5 sphere at
`(width/2, 0, 0.5)`, away from the authored centerline route. Customize it in
**Mesh hazards** after creation. Runtime mission/hazard progress starts at zero.

Creation validates exact save/reopen and host readback, refuses existing scene
paths and orphan presentation sidecars, and uses the no-overwrite scene writer.
Dirty scenes and presentation drafts, Play, gestures, pending path dialogs and
other in-progress operations block creation. Cancel or Escape emits no request;
repeated creation cannot replace an existing file. A write/startup failure keeps
the current editor and may retain a newly created file (including a partial file
on I/O failure), so inspect it before choosing another destination.

**New mesh game…** retains its legacy profile. New hazard creation does not
migrate existing scenes, enable legacy workspace operations for hazard scenes,
or provide hazard asset editing or composed-presentation export.
The separate `mesh-hazards-export` feature supports scene-only standalone hazard
bundles; see [the export guide](editor-mesh-hazards-export.md).
Unsupported workflows remain unavailable rather than silently falling back to
the legacy profile.

Headless tests cover decimal/count validation, bounded geometry, canonical
zero disable, exact readback, Undo/Redo, checksum/epoch/graph stale-draft guards,
Play rejection, legacy-profile isolation, and actual egui Apply/Reload actions.

Creator tests additionally cover explicit profile selection, canonical optional
hazards, no-overwrite/sidecar/path checks, cancellation and mode isolation,
current-scene preservation, and Play through the collect/exit route with the
optional sphere at minimum, default and maximum tested floor dimensions.


## Pick a hazard center in the viewport

Enable hazards and choose a sphere count, then choose **Pick hazard N center in
viewport** in the Mesh hazards panel. Click the admitted walkable surface to
stage that sphere's X/Y/Z. The existing radius text and every other sphere stay
unchanged. The center lies on the clicked surface; numeric Y remains available
after picking for centers above or below the surface.

A successful hit changes draft fields only. Choose **Apply mesh hazards** to
commit one undoable edit; the orange warning bands then show the applied center.
Save explicitly to persist it. A miss leaves picking active so you can try again.
**Cancel hazard center picking** or Escape preserves the existing draft.

While armed, other authoring interactions, Play, Save, export and scene switches
are blocked. Focus loss, text focus, a modal, preview or conflicting content work
cancels picking. A scene-load epoch, graph, document checksum, sphere count or
numeric draft change also invalidates the selection before a click is accepted.
Picking is available only in idle local MeshHazardsV1 Edit mode; it does not
change the legacy mesh profile or implement a second collision/raycaster system.
