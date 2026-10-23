# Animated models in Room projects

Room projects can bind an actor to an animated GLB so its presentation pose is
sampled from the authoritative simulation tick. Animation is a view feature: it
does not change Room gameplay state, physics, checksums, or collision behavior.
Model bindings are stored separately from `room.scene.yaml`, so changing a model
does not rewrite the authored gameplay scene.

## Enable the feature

Animated Room bindings are opt-in. The default Room build remains static-only;
it rejects animated Room bindings rather than silently loading them without
animation support. Build the consumer and editor with the same feature:

```powershell
cargo check -p orr_sample --features room-animated-models
cargo check -p orr_editor --features room-animated-models
```

Create a starter Room project once, then launch the editor with that project.
The creator and editor commands below are Windows-only:

```powershell
cargo run -p orr_sample --features "project-create,room-project" --bin orr_new_room -- --output 'E:\Games\MyRoom' --seed room-demo
cargo run -p orr_editor --features "windows-projects,room-animated-models" -- --room-project 'E:\Games\MyRoom'
```

The feature enables Room project loading, animation-capable model bindings,
animation decoding, and animated rendering. It is not enabled by the default
feature set.

## Add an animated model

1. Open **File > Content packages…** and enter a local package directory.
2. Verify the package, preview its addition, add it, then verify the installed
   package bytes. The package manifest must declare the `animation` capability
   and include the GLB asset.
3. Select a Room actor and choose **Use animated model** for the verified asset.
   This assigns the asset's first clip with loop playback as a starting point.
4. In the model inspector, choose an animation clip and playback mode, then use
   **Assign animated model** to apply that choice. Model-binding changes support
   Undo and Redo.
5. Use **Save model bindings** to persist the binding sidecar, then save the
   Room project. Reopen the project to load the package and binding again.

Package verification checks declared local package content; it does not download
packages or execute package scripts. Exporting a Windows build still requires
the existing trusted-runtime workflow.

## Runtime limits

Room project admission currently bounds animated content to at most 8 distinct
model assets, 68 model bindings, 64 MiB of aggregate decoded model data, and
1,000,000 units of animation work per frame. A project exceeding these limits
is rejected. These limits are safeguards, not a guarantee that every GLB or
animation is supported; assets must also pass the model and animation decoders.

Animation is presentation-only and follows simulation snapshots. Pausing,
stepping, and replaying the same deterministic simulation state therefore do
not introduce separate gameplay state. Exported Room projects require a runtime
built with compatible Room animation support to load animated bindings.

## Windows editor acceptance

The actual EditorApp acceptance flow is in
`crates/orr_editor/tests/room_animated_ui.rs`. It uses the repository's local
animated sample package and a hardware Vulkan adapter to exercise package
verification/addition, assigning a clip and playback mode, Undo/Redo, saving
bindings, reopening the project, tick-based pose sampling, and optionally the
same-App Windows export path. This is an integration check for that workflow,
not exhaustive validation of all assets, devices, or project states.
The flow checks the reopened pose against the chosen clip at sampled
authoritative ticks and checks that it differs from the saved rest pose. It does
not prove every intermediate pose, animation rate, or every clip's behavior.

Run it on Windows x64 with the feature enabled:

```powershell
$env:WGPU_BACKEND = "vulkan"
$env:ORR_ROOM_ANIMATED_ACCEPTANCE = "1"
cargo test -p orr_editor --features "windows-projects,room-animated-models" --test room_animated_ui -- --exact room_animated_package_ui_save_reopen_playback_and_export --nocapture
```

The test also runs when `ORR_REQUIRE_GPU=1` is set. Without an acceptance
environment variable it exits without claiming acceptance. It requires a
hardware Vulkan GPU; a software adapter is rejected. Optional paths are
`ORR_ROOM_ANIMATED_OUTPUT` for the newly created project,
`ORR_ROOM_ANIMATED_CAPTURE_DIR` for UI capture,
`ORR_WINDOWS_EXPORT_RUNTIME` for the trusted runtime executable, and
`ORR_ROOM_ANIMATED_EXPORT_BUNDLE` for an export destination. Provide fresh,
absolute output paths; the export destination requires the runtime variable.
