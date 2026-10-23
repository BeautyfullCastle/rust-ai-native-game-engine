# Terrain ramp authoring

The optional `terrain` editor feature includes a finite, axis-aligned ramp tool.
It composes with the existing scene-owned terrain document; it is not the
Raise/Lower/Flatten/Smooth brush and does not change brush behavior.

## Make a ramp

1. Open a saved local Yard3D or Navigation scene in Edit mode. Create or open
   writable scene-local terrain. Installed terrain packages must first be copied
   to the scene using the existing terrain panel.
2. Set the desired endpoint heights with the existing vertex-height editor.
   Ramp authoring reads those heights, rather than accepting separate floating
   point endpoint heights.
3. Open **Terrain ramp**. Press **Pick ramp start**, then click a terrain vertex
   in the existing viewport. Press **Pick ramp end**, then click the other vertex.
   Picking uses the same terrain vertex selector and screen-space tolerance as
   the existing panel. Misses do not select scene entities. The endpoints must
   be distinct and share X or Z; diagonal and curved ramps are not supported.
4. Set **Ramp half-width (cells)**. A value of 1 means a two-cell-wide corridor
   spanning three vertex rows. Width is never silently clipped to terrain bounds.
5. Press **Stage ramp**. The cyan wire grid is prospective geometry, not the
   applied terrain. Faint vertical lines connect changed existing vertices to
   their prospective heights. Endpoint markers, footprint, endpoint heights,
   total footprint vertices and changed-vertex count accompany the preview.
6. **Cancel ramp** (or Escape) clears only the pending tool state. **Apply ramp**
   changes the terrain in one existing Terrain Undo transaction. An already
   matching candidate adds no history and preserves a pending redo branch.
7. Use **Terrain Undo** / **Terrain Redo** to reverse or restore the entire ramp.
   Use **Save terrain** to persist it. Scene Save alone does not save terrain.

Picking either endpoint again removes the staged preview. Changing width also
requires a new Stage, while retaining the endpoint choices. Observed mismatches in
terrain content, source, scene, host epoch, selected entities, terrain
selection or height/hole settings cancel pending state. Focus loss, Play,
read-only terrain and a proposal preview also cancel it. Apply checks the pinned
context and source again synchronously; a currently mismatched preview is never
applied. These checks compare current canonical content and visible context,
not a monotonic terrain-history event serial. An edit followed by Undo, or an
identical-source reopen, that restores every pinned value between checks is not
detectable through the existing public document API.
Admission failures are displayed and do not replace terrain/history/files.

## Exact geometry contract

The inclusive vertex rectangle runs from one endpoint's axial coordinate to the
other's and from the transverse center minus half-width to center plus
half-width. The entire rectangle must fit. All transverse vertices share the
same axial height profile. The two center endpoint heights remain exact.
Outside-footprint **vertex heights** remain unchanged, as do holes, dimensions,
identity, origin and spacing. Neighboring cells sharing boundary vertices may
change their interpolated surface even outside the vertex rectangle.

Heights are Q48.16 values throughout generation. For axial distance `n`, offset
`t` from the supplied start, and original raw endpoint heights `h0` and `h1`, the
candidate rounds `((n-t)*h0 + t*h1)/n` to the nearest raw integer, ties away from
zero, with signed i128 intermediates. It rounds the absolute weighted rational,
not a separately rounded height delta. Reversing endpoint order therefore
produces exactly the same heights. No floating-point height calculation is used;
float conversion is only for the existing viewport projection.

Any hole in a cell incident to any footprint vertex rejects the ramp, including
the outside one-cell neighbor halo at sides and ends and vertices that would
not change. The incident-cell rectangle starts at
`vertex_min.saturating_sub(1)` and ends at `min(vertex_max, dimension-2)` on each
axis. Unlike the footprint, this incident domain is clipped to existing cells.
The tool does not bridge holes or install a collision surface.

The candidate emits changed-only row-major SetHeight edits. A candidate exceeding
the existing 4096-edit transaction limit is rejected, not split into multiple
Undo steps. Stage and preview never call the document apply path and never
write source files. Apply uses the existing transactional mesh/content admission;
if that rejects the candidate, the admitted source remains intact.

## Navigation and limitations

For the existing point-agent terrain Navigation workflow, use a surface no
larger than 17 × 17 vertices. Save the terrain, explicitly build a new route with
the Navigation panel, save the scene's updated pins, then reopen and Play.
A terrain edit does not rebuild a route automatically; a stale source pin must
be rebuilt before Play. Ramp authoring does not add polygon navmesh generation,
rigid-body terrain collision, live physics deformation, texture painting,
falloff, multiple-terrain joins, diagonal or curved roads.

A finite ramp does not automatically create flat landings. At its endpoints and
sides, incident cells connect the ramp's boundary vertices to the unchanged
neighboring terrain. Those cells may be steeper than the ramp itself and fail the
route's slope limit. Prepare suitable landing terrain, or choose route endpoints
inside the usable corridor, then explicitly build and inspect the route. The
existing route follows triangle-midpoint waypoints; it does not promise a
straight centerline or funnel-smoothed path.

## Inspector composition and compatibility

Yard3D and Navigation use one shared vertical inspector scroller, so terrain,
ramp and subsequent Navigation controls remain reachable together. TerrainPanel
adds a parent-scrolled entry point that reuses the same controls without nesting
its own scroll area. Its existing `show` entry point retains the bounded inner
scroller and existing control behavior for standalone callers.

Ramp calculation and interaction live in new candidate/adapter modules with
narrow editor hooks. The terrain document, picker, navigation logic, core
serialization and independent brush APIs are unchanged. The additive panel
entry point is an explicit overlap with the separately owned PR166 brush work:
that ownership reservation remains in force, not taken over by this feature.
A future PR166 combination must preserve and verify both tools and the shared
scroll composition.

## Verification scope

Local acceptance passed 18 focused CPU tests and one explicit SwiftShader GPU
workflow. The GPU run recorded 15 phases and checked 57 authoritative navigation
Frames, covering interactive authoring, preview/apply, persistence and point-agent
consumption. These results do not certify hosted CI or native Windows behavior.
Original failed attempts remain in the verification evidence. The lifecycle
fixture drains existing pending work using its original timeout; acceptance did
not relax that timeout to obtain a pass.
