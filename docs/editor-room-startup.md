# Start a Room project from the editor

The Windows editor can create or open a Room project from its normal local
Physics startup. You do not need to launch the editor with `--room-project` or
run a separate project-creation command first.

## Start

On Windows x64, build and launch the editor with the `windows-projects` feature:

```powershell
cargo run -p orr_editor --features windows-projects
```

The editor starts in its default local Physics scene. Choose **File > New Room
project…**, enter a new output directory and seed, then choose **Create Room
project**. The editor validates the generated project and switches to its Room
scene when admission succeeds. To load a project instead, choose **File > Open
Room project…**, enter its project directory, and choose **Open Room project**.

The existing command-line route remains available for opening a known project:

```powershell
cargo run -p orr_editor --features windows-projects -- --room-project 'E:\Games\MyRoom'
```

## Safe project changes

New/Open does not discard the current document. An unsaved scene or presentation
sidecar blocks the operation; save it or undo/discard the edits first. Active
playback, gestures, transactions, proposals, captures, dialogs, package work,
or an ERP listener also prevent a switch. Failed validation leaves the current
editor session active, and a failed creation does not publish a partial project.
The startup Physics scene is not modified by creating or opening a Room project.

The candidate Room project is validated before the editor swaps hosts. A
successful switch installs the admitted Room scene, model bindings, and camera.
Afterward, the normal Room workflow supports layout authoring, Save, reopening,
Play/Pause/Step/Stop, and Windows export with the existing explicitly trusted
runtime flow.

## Windows acceptance

The actual EditorApp lifecycle acceptance lives in
`crates/orr_editor/tests/windows_room_bootstrap_ui.rs`. It starts from a copied
local Physics scene, checks malformed Open recovery and dirty-document guards,
creates a Room project through the File menu, edits and saves the layout through
the inspector, opens another project and reopens the saved one, then exercises
simulation controls. An optional same-App export requires a trusted runtime.
This is a bounded integration workflow, not exhaustive coverage of all project
inputs or machine configurations.

On Windows x64 with a hardware Vulkan adapter:

```powershell
$env:WGPU_BACKEND = "vulkan"
$env:ORR_ROOM_BOOTSTRAP_ACCEPTANCE = "1"
cargo test -p orr_editor --features windows-projects --test windows_room_bootstrap_ui -- --exact windows_room_project_bootstraps_from_clean_physics_editor --nocapture
```

`ORR_REQUIRE_GPU=1` also requests acceptance; without an acceptance variable,
the test exits without claiming it. Software adapters are rejected. Optional
absolute paths are `ORR_ROOM_BOOTSTRAP_OUTPUT` for the new project,
`ORR_ROOM_BOOTSTRAP_CAPTURE_DIR` for captured UI and a receipt,
`ORR_WINDOWS_EXPORT_RUNTIME` for the trusted runtime, and
`ORR_ROOM_BOOTSTRAP_EXPORT_BUNDLE` for the export destination. The export
destination requires the runtime variable; output paths must be new.
