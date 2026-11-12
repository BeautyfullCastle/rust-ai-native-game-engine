# NavigationYard viewport endpoint picking

With the optional `navigation` feature, the Point navigation panel can select a route's start or goal from the terrain viewport. This avoids transcribing coordinates while laying out a small point-agent route. Numeric fields remain available for precise arbitrary coordinates.

## Workflow

Create or open local NavigationYard3D terrain and save it. Choose **Pick navigation start** or **Pick navigation goal**, then click the desired terrain cell. Picking updates only the corresponding route draft. Use **Build route** to validate connectivity and commit the route through the existing single host-history edit. Undo/Redo, scene Save, reopening and Play use the existing route admission and simulation.

A successful endpoint pick establishes a walkable endpoint, not a connected route. This remains point-agent heightfield navigation, not polygon NavMesh, collision-free character movement, radius clearance or normal alignment.

## Integer and ownership contract

For each XZ axis the snapped coordinate is `origin_raw + cell_index * spacing_raw + spacing_raw / 2`, evaluated with checked wide integer arithmetic. Positive odd raw spacing rounds the half-spacing toward the lower cell edge. With one raw unit of spacing this is the lower boundary, not an exactly representable geometric midpoint. The displayed decimal round-trips the resulting fixed-point value.

Terrain projection rejects holes and cells excluded by the current slope profile. The existing local saved-terrain and bounded 17×17-vertex/512-triangle admission remains in force. Picking never changes terrain heights, entities, the authoritative Frame, scene files or history. The selected endpoint alone does not rebuild the route.

An armed pick is tied to scene/load and host epochs, terrain source/revision and all route draft values. Context changes, Play, focus loss, Escape and competing authoring operations cancel it. An owned click that fails validation is consumed rather than becoming terrain or entity selection. Escape retains the Editor's existing global deselection behavior. Operation boundaries perform source admission; there is no continuous external-file monitoring or new per-frame graph construction.

## Scope and evidence

The feature belongs to the existing optional Editor navigation integration. Core fixed-point arithmetic, simulation, scene schema, navigation runtime and package dependencies are unchanged. Existing typed route creation and ramp authoring remain supported.

Focused acceptance exercises integer edge cases, refusal and cancellation, actual Editor controls and viewport clicks, pre-Build state preservation, Build/Undo/Redo/Save/reopen and authoritative stepped movement. Offscreen rendering and synthetic pointer events do not establish native Windows window input or physical GPU coverage. Exact execution receipts are recorded separately; this document is not itself a test result.

[Unity's documented click-destination workflow](https://docs.unity.com/en-us/engine/6000.7/script-reference/unityengine/ai/navmeshagent/setdestination) is a workflow comparison only. This engine uses its own existing deterministic terrain graph, not Unity NavMesh APIs or formats.
