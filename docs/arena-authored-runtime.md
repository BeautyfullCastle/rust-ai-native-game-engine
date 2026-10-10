# Run a saved Arena project

This is a bounded standalone-playback slice of #104. It opens the same schema-2
Arena entry as the editor, using the project-local installed package snapshots.
It does not generate a project, add game code, or complete the first finished game.
Optional [saved Korean UI/font presets and authored Restart](authored-project-ui.md)
compose with this route. [Export folders](arena-project-export.md) are a separate tool.

```sh
cargo run --release -p orr_sample --bin arena --features project,input-actions -- \
  --project assets/saved_arena_project --bridge inproc
```

`project` is default-off. It enables scene parsing, package-backed sprites and
small read-only project adapters in `orr_sample`; it adds no production dependency
on `orr_editor`, `orr_edit` or `orr_remote`. `input-actions` remains independently
optional. Without it, the existing WASD/arrows/space controls still work. With it,
`--input-bindings FILE` uses the existing Arena action-map/focus/release adapter.
The existing short-tap limitation remains: the bridge consumes latest held state,
not acknowledged tick-edge events.

The window defaults to the existing threaded bridge. `--bridge inproc` uses the
same authored scene/session on the window thread. Neither route creates the
sample loopback peers or opponent bot. Only local slot zero receives keyboard
input; the second slot is idle. Authored players and other supported Arena
components are baked from the saved scene, not recreated by `Arena::setup`.

## Admission and read-only boundary

Both the editor and standalone runtime use one shared admission implementation:

- Schema 2 and an explicit Arena entry; schema 1 remains metadata-only
- Bounded regular scene/sidecar files, strict scene/document parsing, root
  containment, no symlink/FIFO/device inputs or hidden traversal aliases
- Entire active package-lock verification, including unused packages and projects
  with absent or empty sidecars
- Caller-supplied actual compiled capabilities. The editor retains its complete
  mixed-feature inventory. The standalone route advertises only its sprite
  content support, even if other sample features were compiled
- Existing PNG decode formats and limits, both inactive locomotion clips, all
  region/atlas references, all binding/follow GUIDs and usable drawable positions
- Owned admitted scene bytes and decoded pixels before any host/thread/window/GPU

The shared loader does not install, migrate, modify or save anything. Package
schema, digest authority, installed snapshots and atomic-lock semantics are
unchanged. This is an ordinary bounded file-open boundary, not protection against
concurrent hostile filesystem replacement.

Editor adapters still own undo/redo/save, egui textures, ERP row coherence and
Edit/Play camera restoration. Extraction does not add editor authoring operations
to the runtime. Restart/relaunch makes a fresh session from the admitted authored
initial frame; no project bytes change. With `game-ui` and a declared `entry.ui`,
the menu restarts from the immutable admitted frame/config, never from current
disk or a loopback world. Editor Restart deliberately keeps its saved-scene reload
behavior. Existing external Korean UI/restart routes remain available separately.

## Simulation and presentation

The runtime bakes with the real Arena registry and keeps its generation-aware
`SceneIndex`. Session construction is `PlaySession::from_frame` → `PlayHost` →
existing `InProc`/`Threaded` bridge, matching the local editor's seed 42, 60 Hz,
two slots and existing Arena build/replay identity. The bridge alone derives
held-fire commands. Installing a second session input-command adapter would
fire twice and is intentionally avoided.

Shapes, bound sprite positions, independent per-GUID Region/Clip/Locomotion
cursors and camera follow all use one coherent displayed snapshot. Sprites are
layered over the existing Arena shapes. Region pixel dimensions are multiplied
by the authored `units_per_pixel`; nearest sampling and authored GUID order are
preserved. Atlas switches use contiguous draws, so A/B/A order cannot become
A/A/B alpha composition. No hardcoded local-player replacement, facing flip or
wall-clock clip advancement is applied to this route.

Repeated snapshots freeze. Rewind, changed epoch/sequence/history, changed entity
generation and forward gaps over eight ticks reset cursors. Motion in skipped
intermediate frames cannot be reconstructed. Missing/despawned follow targets
hold the last camera center and never resolve a recycled entity index as the
original GUID. Standalone presentation is read-only and does not alter checksums.

## Deterministic headless smoke and capture

```sh
cargo run --release -p orr_sample --bin arena --features project -- \
  --project assets/saved_arena_project --headless --ticks 30 --hold right

# Same runtime compositor as the window. Fails if GPU creation/readback fails.
# The capture path must be new; an existing image is never overwritten.
cargo run --release -p orr_sample --bin arena --features project -- \
  --project assets/saved_arena_project --headless --ticks 2 --hold right,fire \
  --capture /tmp/arena-authored.png
```

Headless ticks are 0..6000. `--hold` is `idle` or comma-separated
`left,right,up,down,fire`; omitted means idle. Initial/final checksums and final
GUID/complete handle/fixed-point position values are printed. These are bounded
smoke controls, not a replay importer or scripted package hook. Captures are
512×512 and may use a software adapter, whose identity is printed.

`--project` rejects `--sprite-project`, `--game-ui-project`,
`--save-input-bindings`, all relay options and loopback/interpolation options
(`--latency`, `--jitter`, `--remote`, `--tau`) before opening files. Headless
project runs additionally reject wall-clock `--seconds`, input-binding files,
explicit threaded pacing and required audio. Other Arena launch routes retain
their existing behavior.

Within a compatible target environment, copy the built executable plus the
complete project directory, including `.orr`, to run from another location.
System libraries and GPU drivers are still required; they are not bundled.
No source-art lookup, install step or embedded fallback is available. The
relocation evidence here is Linux-only, not an exporter, cross-platform package
or reproducible-binary claim.

## Acceptance and limits

The focused acceptance suite includes shared malformed/path/package admission,
real EditorApp edit/save then initial/per-tick/replay checksum parity, two actors,
focus/release/rebinding/held fire, cursor continuity, generation reuse, restart,
actual runtime-compositor GPU texel/order checks and binary/project relocation
from an empty cwd with repository paths hidden. Set `ORR_REQUIRE_GPU=1` to forbid
GPU skips and `ORR_PROJECT_CAPTURE_DIR` to retain runtime test captures. Set
`ORR_REQUIRE_PROJECT_ISOLATION=1` on Linux to require the copied-binary test
to hide its complete source repository with bubblewrap.

Exact commands, results and earlier failures accompany the reviewed change.
Software-GPU/offscreen evidence is distinct from native-window interaction,
physical GPU and Windows/macOS execution. Those are not claimed by this slice.
Existing editor sprite/project, Korean UI, default and mixed-feature guards and
Arena golden/replay checks remain required. #94 and #104 remain open.
