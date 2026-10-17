# Point-and-click Mesh agent route authoring

The optional `mesh-navigation` editor can stage navigation-agent endpoints by
clicking the actual admitted triangle surface. This is the existing point-agent
profile, not a new physics controller or general gameplay scripting system.

## Create and apply a route

1. From a local Physics editor, use **File → New mesh scene…** or
   **Open mesh scene…**. Create/open a mesh, then **Place mesh agent** once to
   admit its graph and fit the camera. Existing model and mesh import paths work
   alongside the built-in ramp/bridge creator.
2. Open **Static-mesh navigation** and use **Pick agent start in viewport**.
   Click a walkable triangle. Use **Pick agent goal in viewport** to stage the
   goal. Green/red boxes mark staged start/goal coordinates.
3. Picking uses the nearest forward triangle intersection, quantizes into exact
   fixed-point coordinates, and accepts only the authoritative graph's matching
   surface projection. Misses and invalid surfaces do not change the host.
4. The clicks update inspector fields only. **Place mesh agent** explicitly
   validates the complete route and applies one existing ERP singleton edit.
   Use the normal **Edit → Undo / Redo** for that authoritative change.
5. Save the scene explicitly. A fresh Physics editor can open the saved Mesh
   scene, Play, Pause, step and Stop it. With `mesh-export`, use the existing
   source-independent export workflow; a graph-only scene needs scene, manifest
   and the trusted local player executable, not a separate source asset.

**Cancel endpoint picking**, Escape or focus loss disarms picking without
changing staged coordinates or the host. Scene, graph, session, mode or
authoritative document-checksum changes also disarm it, including an external
agent edit that leaves graph geometry unchanged. Play, workspace/path changes,
export and simulation undo shortcuts are blocked while a pick is armed. The
camera can still orbit, pan and zoom before the click. Save or undo dirty prop
edits before picking endpoints. Cosmetic props do not become walkable geometry.

## Boundaries and evidence

All picking math is private editor view code. Simulation commands remain exact
fixed-point values, existing route validation, existing scene pins and ERP
history; no public schema, simulation crate, renderer, reserved Room exporter
or package format changes are needed. Graph display bounds stay at ±256 units;
ray/camera inputs are separately bounded. Draft overlays remain Edit-only and
are not saved into the player.

Windows acceptance exercises production EditorApp pointer events and hardware
Vulkan: stage/cancel endpoints without host mutation, explicit placement and
Undo/Redo, Save, fresh Physics-editor Open, 16 Play-mode steps and optional
three-file export. Native relocated-player execution is separate evidence.
Synthetic test input is not physical OS input. Scope/test counts do not imply
whole-PR acceptance, development integration or full-engine completion.
