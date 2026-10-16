# Optional terrain-triangle navigation view lab

This is a bounded, partial #103 prototype: a point agent travels over the existing
canonical `orr_terrain` triangle graph. It is **not full navmesh support**. The
lab exercises the CPU graph, cooked dependency-bound assets, real offline package
installation, explicit stale-path failure, and an offscreen GPU presentation.
There is no scene, physics, editor, input loop, window, or avoidance integration.

`orr_navigation_view` has no GPU dependency by default. Its `gpu` feature adds
`orr_render::ModelRenderer` unchanged. No baseline simulation/render APIs are
modified and no simulation crate depends on this view crate.

## Actual pipeline and required GPU acceptance

```sh
cargo test --release --locked --offline -p orr_navigation_view
ORR_REQUIRE_GPU=1 cargo test --release --locked --offline \
  -p orr_navigation_view --features gpu -- --nocapture
ORR_SOURCE_SHA="$(git rev-parse HEAD)" cargo run --release --locked --offline \
  -p orr_navigation_view --features gpu --bin navigation_lab -- \
  /tmp/navigation-lab-new
```

The lab output directory must not already exist. The runnable binary always
requires a working adapter. The regression test may skip a missing adapter for
ordinary CPU-only development, but **release acceptance requires
`ORR_REQUIRE_GPU=1`, zero ignored tests, and a successful actual readback**. Missing
adapters fail under that setting. Software Vulkan is accepted, with adapter name
and `software_adapter` explicitly recorded; this is offscreen rendering and must
not be represented as physical/windowed play.

For readback-test screenshots, set `ORR_NAVIGATION_SCREENSHOTS` to a new directory.
The binary always writes five actual PNGs, five corresponding JSON reports, and
an aggregate `report.json`:

1. `01-initial`: loaded scenario, cyan portal-midpoint route, orange start agent
2. `02-midtick`: six fixed-point half-unit ticks, orange agent at its new position
3. `03-arrival`: exact loaded scenario goal, bounded by a 4096-tick lab limit
4. `04-edit-stale`: one opening cell removed, old path visibly red, advance fails
   without mutating the agent or its active path
5. `05-replan`: freshly installed cooked graph accepted explicitly, cyan alternate
   route, movement resumes from the same agent XZ

The JSON preserves exact raw Q48.16 agent positions, tick/status, corridor keys,
waypoints, costs, profile, terrain identity/revision, cooked navigation revision,
active path dependency revisions, package digest, loaded scenario identity, GPU
adapter identity, and optional `ORR_SOURCE_SHA` provenance. Each screenshot is one
`StaticModel` containing terrain, path, and agent primitives, drawn by one
`ModelRenderer` call. Independent renderer calls clear color/depth, so the lab
never claims that separate draws provide an overlay.

## Verified package boundary

The real `orr_package::Project` installs three declared assets: `lab.orrt`,
`lab.orrnav`, and `scenario.json`. The consuming host then reopens with its
compiled `terrain_v1` and `navigation_v1` capabilities and loads all three solely
through verified `Project::read_asset` bytes. Capabilities are supplied by compiled
code, never trusted from source/package claims. Scenario coordinates and profile
parameters are raw fixed-point integers, not float-roundtripped JSON values.

Logical terrain, navigation, and scenario asset identities remain stable after
editing. Terrain revision, cooked graph revision, scenario dependencies, package
version, and content-addressed package digest change with content. Cooked graph
load binds terrain identity, canonical terrain revision, and supported profile;
scenario load additionally checks its declared terrain and graph revisions.

CPU tests and the runnable lab's separate disposable negative-boundary projects
prove that missing packages/assets, missing compiled capabilities, installed
cooked-graph tampering, and a valid-hashed package containing a graph for the
wrong terrain are rejected. Source mutation after installation cannot alter
loaded navigation, queried terrain, or rendered state. The binary checks edit
undo/redo before installing the new package. Tampering is confined to its new
fixture-local evidence project, leaving the primary installed project usable.

The required GPU test asserts terrain and route pixels survive the shared pass,
hole pixels remain empty, the marker leaves its old position and appears at the
new simulated position, arrival is visible at the goal, the stale path is red
while the agent freezes, the replanned path avoids the newly removed cell, and
post-install source edits do not affect deterministic readback. Viewport resize
and reversal are also covered.

## Deliberate bounds and omissions

- The profile is a point agent: radius, headroom, and max-step must be zero;
  unsupported nonzero values fail in the core instead of being silently ignored
- Walkability uses exact terrain-plane rise/run limits; portals are actual shared
  edges, never diagonal/corner-only adjacency
- The path is a deterministic triangle corridor with portal-midpoint polyline;
  there is no funnel, smoothing, clearance baking, obstacle carving, or avoidance
- Terrain edits require a full graph rebuild/reinstall and explicit agent replan
- Presentation requires exact f32 coordinates within the existing model's
  +/-1,000,000 range; the small lab's arbitrary raw movement coordinates satisfy
  this. Unsupported large/subprecision display coordinates fail explicitly;
  core fixed-point values and evidence are never modified to fit the display
- Tick, edit, stale-color change, and replan require immutable model rebuild/upload
- Existing opaque diffuse/checker materials only; no LOD, streaming, splat layers,
  or claims of renderer/scene integration
