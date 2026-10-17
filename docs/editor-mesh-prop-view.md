# Direct viewport placement of Mesh props

Build the editor with the optional `mesh-navigation` feature. In a saved local
Mesh scene, use **Mesh props** to load a licensed model package candidate and add
a static prop. No package install, network request or script execution is needed.

## Move and save

1. Enable **Move props in viewport** in the Mesh props inspector.
2. Click a prop to select its transformed bounding box, or drag it with the left
   mouse button. The nearest world-space axis-aligned box is picked; equal hits
   use the lowest stable prop ID. This is bounding-box picking, not triangle or
   collider picking.
3. Dragging moves the prop on its original horizontal plane, preserving its Y
   height and initial cursor grab offset. Enable **Snap movement to 0.25 units**
   to snap X/Z to the absolute quarter-unit grid. Edit Y or scale in the inspector.
4. Release to create one entry in the existing private prop history. Use
   **Undo prop edit** and **Redo prop edit**; the simulation's ERP history is not
   changed. A no-motion gesture does not add an entry.
5. Press Escape or lose focus to cancel an active drag and restore its original
   placement without a history entry. A scene/graph/session change also cancels.
6. Click **Save props** explicitly. Unsaved movement does not change the sidecar
   or scene file. Reopen the scene to use its embedded prop data without the
   original package. Play uses the saved placement; the optional `mesh-export`
   workflow copies the admitted sidecar into a source-independent player bundle.

During a drag, Save props, history actions, Play, path/workspace changes and
export are blocked. Orbit, pan, zoom and simulation undo shortcuts cannot alter
an active drag. Selection outlines appear only while this tool is enabled in
Edit mode and are not saved or shown by the standalone player.

If Save As finds an existing prop sidecar, saving carried props does not
overwrite it. Confirm **I confirm discarding unsaved prop changes**, then use
**Discard unsaved props and reload** to discard only the in-memory edits and
admit the current scene's stored props. Neither sidecar is deleted or rewritten.
Saving an empty, clean prop document under another scene path also loads that
target's existing sidecar instead of ignoring it. Ordinary unsaved changes and
stale graph/scene edits use the same explicit discard recovery.

## Scope and verification

Props remain static cosmetic presentation. Moving them does not modify graph
geometry, the navigation agent, scene checksums, ECS entities, physics or
colliders. The existing six-prop and embedded-art admission limits still apply.
No public schema, simulation API, Room exporter or renderer is changed.

Windows acceptance uses the production EditorApp on hardware Vulkan and actual
egui pointer events, not a second sample or a prop-backend shortcut. The workflow
covers drag math, snap, one-release undo/redo, cancellation, explicit saving,
source-hidden fresh reopen, Play and optional export. Native relocated-bundle
execution is recorded separately. Synthetic UI events are not physical OS input.
Implementation, local acceptance, publication and development integration are
reported separately; this workflow is not a full-engine completion claim.
