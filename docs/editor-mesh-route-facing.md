# Mesh agent route-facing art

The MeshNavigation editor can orient static agent art and animated agent art along the agent's admitted navigation path. This is a presentation option: it does not add physics, collision, or a simulation component.

## Use it

1. Create or open a local MeshNavigation scene, author a walkable mesh, place the point agent, and save the scene.
2. In **Mesh agent art**, load a static model from a trusted local package. Keep **Face route direction** unchecked for the legacy authored orientation, or check it to align the model to the route. Use **Apply agent art**, then **Save agent art**.
3. Optionally load an animated model in **Mesh agent animation**. Choose its clip, rate, loop behavior, and authored rotation. Check **Animation face route direction**, then **Apply mesh animation** and **Save mesh animation**.
4. Use each panel's Undo/Redo controls to review edits before saving. Reopen the scene to confirm the saved presentation.

The setting is stored in the existing scene-bound presentation sidecars: `facing: "route"` in the static art sidecar and `settings.facing: "route"` in the animation sidecar. The default fixed orientation is omitted for backward compatibility. Applying, undoing, saving, or reopening these presentation choices does not change the scene checksum or navigation graph revision.

## Orientation behavior

For a route-facing presentation, the editor selects the horizontal path segment nearest the agent's current route distance and composes its world-Y yaw before the model's authored quaternion rotation. Equal-distance segment choices prefer the later segment. If the admitted route has no usable horizontal segment, presentation falls back to the authored rotation. This is derived from the current admitted path and agent readback; it is not a stored runtime quaternion or a guarantee that every intermediate animation pose is valid at every frame rate.

## Build and export

`mesh-navigation` enables static mesh navigation and static art. `mesh-animated-models` adds animated mesh art. The independent `mesh-export` feature enables the standalone Mesh game exporter/player. A bundle that contains route-facing art requires a player built with the matching presentation capability; older replay-only players are rejected rather than silently dropping the setting. Exported art remains a bounded scene sidecar, so the original package directory is not needed after authoring and saving.

Use only trusted, stable local package directories. Package provenance in a sidecar records the inspected source at authoring time; it does not prove current installation, ownership, or authenticity. The feature is cosmetic and does not make the point-navigation agent a character controller.
