# Saved Korean game UI

Schema-2 Arena projects may opt into the closed `arena-korean-v1` presentation
preset on their existing `entry`:

```json
{
  "schema": 2,
  "engine": "^0.0.1",
  "entry": {
    "game": "arena",
    "scene": "arena.scene.yaml",
    "sprites": "arena.sprites.json",
    "ui": {
      "profile": "arena-korean-v1",
      "font": {
        "package": "korean-game-ui",
        "asset": "OrreryKoreanUI.otf"
      }
    }
  }
}
```

The UI reference identifies content; it does not select or install a package.
`orr.packages.lock.json` remains the sole activation and exact-version authority.
Install the local data-only font package through the existing package manager,
then save the reference only when `korean-game-ui` is present in the verified
active lock. The saved sample Arena project also has `sample-sprites`; it retains
both direct selections exactly once. Runtime admission verifies the entire lock,
reads the declared font asset through `Project::read_asset`, checks the font
parser and UI text corpus, then owns the verified font bytes. It creates no
fallback font, host-system font lookup or second package graph.
The checked-in `assets/saved_arena_project` remains a no-UI sample until the
font package is installed and the shown `entry.ui` reference is saved. The
runtime commands below use `/path/to/ui-enabled-project`, meaning a project
with both that metadata and the verified active `korean-game-ui` package.

## Runtime and editor support

The standalone Arena route opts into authored UI only with `game-ui` compiled:

```sh
cargo run --release -p orr_sample --bin arena \
  --features project,input-actions,game-ui -- \
  --project /path/to/ui-enabled-project --bridge inproc

# Headless scene plus the same software-capable project compositor capture path
cargo run --release -p orr_sample --bin arena \
  --features project,game-ui -- \
  --project /path/to/ui-enabled-project --headless --ticks 0 \
  --capture /tmp/saved-arena-korean-ui.png
```

A host without explicit UI support rejects a project that declares `entry.ui`,
including on a zero-tick launch. Merely compiling unrelated sample features does
not implicitly enable the preset. In menus, local-player controls are blocked
while the simulation and network continue. Authored Restart creates a fresh
session from the immutable admitted scene/config rather than current disk state.
Queued pointer moves and clicks are checked against the completed HUD layout
before publishing held input, so a between-frame Menu click cannot become
remapped mouse-fire. Resize/DPI/resume invalidates those bounds and keeps local
controls neutral until the next UI layout; simulation continues throughout.
The editor's separate opt-in is:

```sh
cargo run --release -p orr_editor --features project-ui -- \
  --project /path/to/ui-enabled-project
```

Its inspector shows a read-only preset/font preview. That preview does not create
game widgets over the editor viewport or add UI authoring controls. Editor
Restart keeps its existing saved-scene reload behavior.

In the game window and headless readback, the UI mesh uses the existing SDR
color view. Composition is Arena shapes, globally ordered project sprites
(including contiguous A/B/A atlas runs), and then the UI overlay on that same
view. If surface acquisition is skipped, pending UI texture uploads and
free/reuse epochs are retained until a view is acquired. Resize/DPI uses egui's
point-space coordinates and clip rectangles; the current pixels-per-point scales
them against the resized physical target.

## Export requirements and acceptance

The bounded Linux x86-64 export tool does not build code. Both the exporter and
the supplied already-built trusted `arena` runtime must include `game-ui`; the
tool copies the active package closure unchanged, including `OrreryKoreanUI.otf`,
`font-manifest.json`, `corpus.txt`, `OFL.txt`, `COPYRIGHT.txt` and the package
manifest. Keep the font's OFL-1.1 terms and notices when distributing it. This
closure is not a complete system-library or third-party distribution notice
bundle. See [the export contract](arena-project-export.md) for trust, hash,
staging, relocation and other distribution limits.

Focused tests cover real software-GPU overlay composition, distinct Korean
glyphs, A/B/A alpha order, 1x-to-2x target resize, clipping, queued free/reuse,
deterministic sorted export manifests, exact font closure, relocated launcher
execution from an empty working directory, source hiding when bubblewrap works,
and rejection by a separately built project-only runtime. To retain visual
readback PNGs, create an empty directory first; the tests create new files and
never overwrite existing ones:

```sh
mkdir -p /tmp/project-ui-captures
ORR_REQUIRE_GPU=1 \
ORR_REQUIRE_PROJECT_ISOLATION=1 \
ORR_PROJECT_UI_CAPTURE_DIR=/tmp/project-ui-captures \
cargo test --release -p orr_sample --features project,game-ui \
  --test project_ui_compositor

# Build a second runtime with no game-ui feature, then test that copied binary
# against a valid saved-UI project and as the exporter's supplied trusted runtime.
cargo build --release -p orr_sample --no-default-features \
  --features project --bin arena --target-dir /tmp/orr-project-only-build
cp /tmp/orr-project-only-build/release/arena /tmp/project-only-arena
ORR_REQUIRE_GPU=1 \
ORR_REQUIRE_PROJECT_ISOLATION=1 \
ORR_REQUIRE_PROJECT_ONLY_RUNTIME=1 \
ORR_PROJECT_ONLY_RUNTIME=/tmp/project-only-arena \
ORR_PROJECT_UI_CAPTURE_DIR=/tmp/project-ui-captures \
cargo test --release -p orr_sample \
  --features project-export,game-ui --test project_ui_export
```

Set `ORR_EXPORT_HIDE_ROOT` to the workspace root if it is broader than the
repository checkout; the isolated relocated run verifies that its project,
runtime source and checkout paths are unavailable to the copied launcher.
These checks establish headless/offscreen output only. They do not establish
native-window input, physical-GPU hardware behavior, IME composition, Jamo
shaping quality, accessibility, Windows/macOS support or full distribution
compliance.
