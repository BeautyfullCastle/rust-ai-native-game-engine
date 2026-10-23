# Standalone Mesh Player

The optional `mesh-export` editor feature adds **File → Export mesh game** for an admitted local MeshNavigation scene. Export writes a bundle containing the scene, optional saved agent-art sidecar, a strict `orr.mesh-bundle.json` inventory, and the `orr_mesh_player.exe` runtime. The player is a separate executable; it does not open the editor or expose scene-authoring controls.

The editor feature and player binary are opt-in:

```powershell
cargo build --release -p orr_editor --features mesh-export --bin orr_editor --bin orr_mesh_player
```

Run `orr_mesh_player.exe` from the exported bundle directory, or pass `--bundle DIR` to select a bundle explicitly:

```powershell
& 'E:\Games\MyMeshGame\orr_mesh_player.exe'
& 'E:\Games\MyMeshGame\orr_mesh_player.exe' --bundle 'E:\Games\MyMeshGame'
```

Without `--bundle`, the player uses the directory beside its running executable. There is no default-scene fallback. Before starting, it admits the bundle and rechecks that the running executable's SHA-256 matches the bundle inventory. A mismatch, missing file, invalid scene, or incompatible graph fails closed. Bundle validation and playback do not execute package scripts, install packages, access a remote host, or write into the bundle.

The runtime exposes `ORR_MESH_PLAYER_V1` as a dedicated `.orrmesh` PE data section, which the exporter checks without executing the player. This marker identifies the player interface; it is not a code signature, publisher identity, or authenticity guarantee.

The limited command-line options are:

| Option | Behavior |
|---|---|
| `--capabilities` | Print exactly `ORR_MESH_PLAYER_V1` and exit without opening a bundle. |
| `--bundle DIR` | Use this validated bundle directory instead of the executable's directory. |
| `--ticks N` | Start the simulation and leave it paused at exactly `N` ticks; `N` is limited to 10,000. |
| `--screenshot PNG` | Save a window screenshot after the requested frame count; requires an existing output parent, rejects an observed existing filesystem entry, and writes outside the bundle. Without `--ticks`, screenshot mode starts paused at tick 0. |
| `--frames N` | Screenshot frame count, 1–10,000; requires `--screenshot` and defaults to 30. |
| `--size WxH` | Initial window dimensions in points, constrained to bounded dimensions and at most 1,048,576 pixels; defaults to `960x600`. |

The in-game controls pause, resume, and replay the admitted simulation. MeshNavigation is a static-triangle graph with a point navigation agent; it is not a general character controller or full collision/physics runtime. A successful bundle admission verifies the exported scene and graph snapshot, not every possible runtime condition, hardware, or intermediate position. Bundle hashes detect changes relative to the manifest; they are integrity checks, not publisher authentication or a digital signature. Screenshot preflight rejects existing entries as observed, but path-based checking is not a race-safe no-replace guarantee if another process changes the destination concurrently.
