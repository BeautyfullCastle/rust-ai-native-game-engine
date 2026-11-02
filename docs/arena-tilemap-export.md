# Export an authored Arena tilemap (Linux CLI)

An optional `project-export,tilemap` build can export an already saved Arena
project with its visual background tilemap. This connects the existing authored
runtime to the existing Linux x86-64 exporter; it does not add a tilemap decoder,
change simulation, or enable optional features automatically.

```sh
cargo build --locked --release -p orr_sample --features project-export,tilemap \
  --bin arena --bin orr_export_arena
sha256sum target/release/arena
# Substitute that measured hash and existing trusted absolute paths below.
target/release/orr_export_arena --project /games/my-arena \
  --runtime /trusted/arena --runtime-sha256 MEASURED_SHA256 \
  --trusted-runtime --output /exports/new-arena
/exports/new-arena/run-arena --headless --ticks 60 --capture /captures/new.png
```

The project must already pass ordinary tilemap admission: its root-level sidecar,
installed sprite document/atlas, package declarations and hashes, region geometry,
and combined actor/tile/atlas bounds. The tilemap is visual-only, not collision or
navigation. Export never strips `entry.tilemap` to make a project pass.

The export snapshots the exact admitted tilemap bytes and source path. The file
is included in the existing preflight byte/file/path budgets, content digest and
source rechecks. Atlas and descriptor bytes travel in the existing verified
package closure. Missing, malformed, stale or tampered input fails before final
publication. The copied project is reopened and the supplied runtime actually
runs the existing smoke check before the new destination is published. A runtime
compiled without tilemap support rejects the project; an ELF header/hash alone
is not a capability or trust certificate.

Supply only trusted, already-built runtime code. Existing export path checks,
new-output-only publication and ordinary source-change detection remain in force;
this is not a sandbox against hostile concurrent filesystem changes. An existing
destination is never replaced. Source projects and package stores are not edited.
The relocated launcher reads its adjacent `project/` directory.

## Boundaries and acceptance

The editor export button still refuses tilemap projects. This is CLI support,
not native editor interaction, Windows export or physics tilemaps. Historical
Paint/Fill/Resize acceptance in `tilemap-authoring.md` remains historical; it
never certified an exported player.

Focused tests cover private snapshot admission and recheck, actual creator and
exporter CLIs, corrupt/missing input, relocation, and existing-output preservation.
The separate explicitly selected GPU test consumes the relocated runtime's real
`--capture` output, compares nonempty map pixels with original atlas texels and
no-map/wrong-region controls, and checks matching simulation output. It hides
owned original source paths by renaming them and launches from an empty working
directory: ordinary relocation evidence, not hostile-process sandbox isolation.
The test requires its documented binary/evidence environment and a real adapter;
unrun or unavailable GPU capture is not a pass. Report adapter and terminal
results separately from CPU checks; no hardware-wide or physical-input claim.

Older generated starter README files say export is unsupported; that statement
predates this CLI path. Their editor-button limitation still applies.

Headless capture uses the same background-before-actors composition as the native
player; optional game UI paints afterward. Explicit GPU acceptance also runs with
`game-ui` compiled and with the authored Korean UI/font fixture, using UI/no-map
and no-UI/map controls. Set `ORR_REQUIRE_GPU=1` and existing absolute evidence
directories `ORR_TILEMAP_EXPORT_EVIDENCE` and
`ORR_TILEMAP_UI_EXPORT_EVIDENCE` when selecting these ignored tests. Captures do
not certify native input, display hardware or font readability on every platform.
