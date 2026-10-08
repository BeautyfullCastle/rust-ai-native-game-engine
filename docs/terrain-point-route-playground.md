# Terrain point-route playground

This optional creation slice connects the existing deterministic NavigationYard3D
point runtime to an owned project, editor route, standalone player and Linux export.
It does not implement a WASD character, clearance-aware navmesh, funnel, climbing,
crowds or sphere/terrain physics. Radius, headroom and maximum step remain zero.

## Build and create

```sh
cargo build --release -p orr_sample --features navigation-project,project-create,project-export \
  --bin orr_new_navigation --bin navigation_playground --bin orr_export_navigation
cargo build --release -p orr_editor --features navigation-project,project-create
orr_new_navigation --output /absolute/new-project --seed example
orr_editor --navigation-project /absolute/new-project
navigation_playground --project /absolute/new-project
```

The generator writes four owned files: `orr.project.json`, `navigation.scene.yaml`,
`terrain.orrt`, and `README.md`. It neither installs a package nor creates a lock.
The terrain has a 0.25 slope and a central hole barrier forcing a visible detour.
The seed changes the terrain identity deterministically; simulation remains seed42,
60Hz, two idle input slots and the existing build identity. Repeating template/tool/
seed reproduces bytes. Existing package terrain imports still use the editor's
explicit immutable-package-to-owned-scene copy workflow.

## Edit and play

Open **Terrain authoring** and **Point navigation**. Open `terrain.orrt`, edit
heights or holes and Save terrain. Adjust start, goal, maximum slope and distance
per tick, explicitly **Build route**, then Save the scene. A source or route draft
change is stale and blocks Play. Saving a changed source and closing the terrain
panel does not clear that stale state. A failed Build preserves the previous
admitted scene/Frame. Undo, Save, reopen and the existing Play/Pause/Step/Seek/Stop
controls remain the authoritative editor workflow.

The editor's initial host consumes captured project bytes. Host restart reads the
current closed project again; it never silently changes to loose-scene admission.
The original scene directory remains the pin's filesystem authority.

In the standalone player, Space/P pauses or resumes and R restarts from the
admitted immutable initial Frame and view. Mouse orbit/pan/zoom affects only the
view. There is no movement input: the point automatically traverses its route.
All ticks, seeking and restart are filesystem-free after admission.

```sh
navigation_playground --project /absolute/new-project --headless --ticks 24
navigation_playground --project /absolute/new-project --headless --ticks 24 \
  --capture /absolute/new.png --capture-size 960x720 --software-gpu
navigation_playground --project /absolute/new-project --headless \
  --restart-after 37 --ticks 24
```

## Export

```sh
orr_export_navigation --project /absolute/new-project \
  --runtime /absolute/navigation_playground --runtime-sha256 VERIFIED_SHA256 \
  --output /absolute/new-bundle --trusted-runtime
/absolute/new-bundle/run-navigation-playground
```

The supplied executable must be explicitly trusted and already built. A matching
hash and smoke output do not establish executable trust. The exporter copies the
exact manifest, scene-relative pinned terrain and runtime, using its own domain
and profile. No build, download or installation runs. Output publication never
replaces an existing destination. Relocation, source-hidden read-only execution,
restart and tamper-before-startup are explicit acceptance gates.

## Boundaries

- Schema2 profile `terrain-point-route-3d-v1` rejects UI, camera, model, sprite and
  progress descriptors. Metadata alone never enables a consuming feature
- One Frame-owned point route, at most17×17 terrain vertices/512triangles
- Full terrain identity, terrain revision and graph revision must match the pin
- Closed project presentation coordinates stay within XYZ±256. Legacy headless
  navigation's wider bounds are unchanged
- Startup rejects malformed, missing, oversized, traversal, symlink and special
  source files; normal source changes are rechecked. This is not a hostile
  concurrent filesystem-replacement sandbox
- Shared terrain-free overlay geometry and one shared depth pass are used by both
  editor and standalone view. Standalone uses fitted orthographic projection;
  editor retains its perspective orbit. Cross-consumer Frame/navigation checksums
  are exact; source/export images are byte-identical at matching capture settings
- Core ECS/simulation and default physics are unchanged. `navigation-project`
  enables optional host/view adapters without a new external dependency. Existing
  `project` transitively enables sprite dependencies even though this consumer
  rejects sprite metadata
- Software-GPU/headless acceptance does not prove native OS-window operation,
  physical GPU behavior or Windows support. Those require separate actual runs

## Acceptance status

Implementation and test sources are in progress. Do not infer passed verification
from this guide. Exact source review, positive-count checks and fresh required CI
must precede development integration. Main and PR1 are excluded; #94/#103 remain
open for their broader scope.
