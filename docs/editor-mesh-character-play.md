# Mesh character presentation and follow camera

Mesh Navigation can present an admitted point-navigation agent with a saved animated model, route-derived facing, idle/moving clip selection, and a saved follow camera. These are presentation settings. They do not add character physics, collision, combat, animation events, or a new scene component.

## Author the presentation

1. Build the editor with `mesh-animated-models` and `mesh-export`, then create or open a local Mesh Navigation scene.
2. Author and place a walkable mesh agent, then save the scene.
3. In **Mesh agent animation**, choose a trusted local package, declared animated asset, and license file. Review distribution rights separately, acknowledge the prompt, and load the candidate. Choose a moving clip and optionally enable **Use idle and moving clips** with a valid idle clip index. Enable **Animation face route direction** if the model should face along the admitted route. Apply, review with Undo/Redo, and explicitly save the animation sidecar.
4. In **Mesh game camera**, enable **Follow mesh agent**, set the world-space offset, apply, review with Undo/Redo, and explicitly save the camera sidecar.
5. Reopen the scene and use Play. The saved camera follows the agent; the animation panel reports the selected moving or idle clip from the admitted agent state.

The camera offset is presentation-only. During Play, the followed target is derived from the agent's current admitted position and the saved offset, while preserving camera pan relative to the authored target. During Edit, camera navigation remains transient unless explicitly captured and saved.

## Timing and compatibility

The animation samples the authoritative host Frame tick; there is no independent animation clock or blending between idle and moving clips. The moving clip is selected while the agent status is moving; the idle clip is selected while stopped or arrived. Route-facing yaw is derived from the currently admitted path and falls back to authored orientation when no usable horizontal segment exists. These checks do not prove every intermediate pose at every frame rate, support animation events, or synchronize animation with collisions.

The animation and camera settings are stored in scene-bound sidecars. A saved animated model is embedded, so reopening does not require the source package directory. Package provenance records the inspected source at authoring time; it is not authentication or proof of current installation. Package scripts are not run and external model resources are rejected. A license file or UI acknowledgment is not legal review. Use trusted, stable local directories; filesystem hash checks do not isolate concurrent writers.

## Export

Build the editor and player together with `cargo build --release -p orr_editor --features mesh-animated-models,mesh-export --bin orr_editor --bin orr_mesh_player`. **File > Export mesh game…** includes the saved scene, animation sidecar, camera sidecar, manifest, and matching player. The player validates the bundle and renders the admitted mesh-navigation agent; it is not a general-purpose character runtime and does not provide physics, combat, or action input.

The saved animation idle/moving option and follow camera require matching player presentation capabilities. Older players reject unsupported presentation metadata instead of silently discarding it. Bundle hashes detect byte mismatches but do not authenticate an untrusted publisher.
