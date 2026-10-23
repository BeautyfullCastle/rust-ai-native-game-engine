# New Mesh game

The editor's **New mesh game** wizard creates a new local Mesh Navigation scene from a small set of starter-game settings. Enter an absolute `*.scene.yaml` path, floor width and depth, collectible count, and fixed-point distance per tick, then choose **Create mesh game**. The scene is published only after the workspace creator validates the settings and prepares the new game. Existing scene paths are not overwritten; the parent directory must already exist.

If creation is refused, the wizard stays open with the path and settings intact so you can correct them. Use **Cancel new mesh game** to leave without creating a scene. Workspace admission and settings validation are authoritative; the dialog does not clamp invalid values into a different game.

The generated floor and collectibles are a starter for point navigation on an authored mesh. This workflow does not provide rigid-body physics, collision response, or physics-driven character movement. Use the Mesh Navigation controls to author the walkable surface and point agent, save the scene, and then continue authoring its presentation and mission state through the editor.
