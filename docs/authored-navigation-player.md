# Authored NavigationYard3D player (optional)

`orr_navigation_view` now provides a default-off Linux `authored` consumer for the
NavigationYard3D scene saved by the editor. This first slice is native headless
simulation and optional offscreen capture, **not** a window/input game, bundle
publisher, general navmesh, physics controller or avoidance system.

```sh
cargo build --locked --release -p orr_navigation_view --features authored --bin navigation_player
navigation_player --scene /trusted/project/navigation.scene.yaml --ticks 60 --restart-check
# Optional existing renderer; GPU/software Vulkan adapter required:
cargo build --locked --release -p orr_navigation_view --features authored,gpu --bin navigation_player
navigation_player --scene /trusted/project/navigation.scene.yaml --ticks 60 --capture /trusted/new.png
```

## Author, save and run

In the NavigationYard3D editor, create/edit terrain, explicitly Save terrain,
author the point route, Build route, and Save the scene. Unsaved panel text does
not become player input. Copy the saved scene together with its **same relative
terrain path** when moving a project. No export bundle or runtime executable is
created by this command.

The consumer reuses `LocalNavigationAdmission` and the existing scene bake:
terrain identity and complete terrain/graph revisions must exactly match the
saved `NavigationScenePin`. Empty edit-only staging scenes are not playable.
Authored entities/bodies/colliders remain forbidden; admission creates the one
runtime point agent. Terrain is bounded to 17×17; radius, headroom and max-step
clearance remain exactly zero. This is not finite-radius path clearance.

The initial admitted Frame is immutable. Neutral input advances the actual
`NavigationYard3D` Simulation at existing YARD tick-rate/build/player settings;
authoritative failure status is checked before reporting/rendering success.
Restart uses the retained initial Frame, not a fresh filesystem read. Hiding or
changing sources after admission cannot alter that run/restart; opening a new
player performs admission again and rejects missing or changed pinned sources.
`--restart-check` compares a repeated trajectory from the same initial state.

## Limits and output

Scene text is at most 1MiB, its OS path at most 4KiB, arguments at most
16KiB, and `--ticks` is in 0..=100000. Capture is fixed at 1000×750 pixels.
Inputs must be local regular files with no symlink ancestry. The reused admission
uses metadata checks and ordinary opens: inputs and ancestry must be trusted and
quiescent. This is **not** a hostile concurrent filesystem sandbox or an atomic
project snapshot. No network, host server or editor mutation command is started.

The optional capture uses the existing terrain/path/agent model renderer and
never feeds display floats back into simulation. Unsupported exact-f32 display
coordinates fail rather than rounding the authoritative state. A fixed bounded
image is written only to a new output; existing outputs are never overwritten.
If output writing/sync fails, inspect the retained partial/uncertain result;
do not assume rollback or blindly retry. JSON reports CPU state, not a gameplay
or GPU success certificate when capture is absent.

Validation distinguishes actual Editor Save/API→player Frame parity, standalone
relocation/source-hidden restart, and explicitly executed optional GPU capture.
Editor widget clicking, physical OS input, Windows/native-window gameplay and
bundle source isolation are not implied by headless tests. Existing frozen
candidate's exact 83-file inventory remains unavailable; original candidates
and core simulation/navigation/schema are not modified.
