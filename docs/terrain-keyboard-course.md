# Terrain Trail: a small keyboard course

Terrain Trail is an optional, original point-anchor traversal sample. Explore a
green asymmetric hillside as a blue explorer, collect four numbered gold beacons
in order, then reach the summit flag. The 17×17 terrain has a visible 2×2-cell
hole. This is an intentionally small playable course, not a terrain editor,
project exporter, physics character controller or navigation system.

## Run

```sh
# Default package is CPU-only: load and report the embedded immutable snapshot.
cargo run --locked -p orr_terrain_demo --bin terrain_demo -- --headless
# Explicit existing renderer/window dependencies; no editor, egui or audio.
cargo run --locked -p orr_terrain_demo --features window --bin terrain_demo
# Read an authored course JSON and its sibling canonical terrain file once.
cargo run --locked -p orr_terrain_demo --features window --bin terrain_demo -- \
  --course crates/orr_terrain_demo/assets/hill_course.json
```

- WASD or arrows: world-relative movement; W/Up is −Z, A/Left is −X
- Left mouse drag: orbit the camera; wheel: zoom
- R: reset explorer, progress, held keys, timing and camera to the loaded baseline
- Escape: exit
- Window title: current beacon, progress, completion and controls

The next beacon is larger/brighter. Completed beacons turn green; the summit flag
changes when it becomes the goal and again on completion. Completion freezes the
run until restart. The explorer is artwork anchored at one sampled point; it can
overhang an edge and has no collision volume. Seven-segment beacon numbers are
depth-tested world geometry and can be edge-on or hidden. The title is the
progress fallback.

The following camera tracks the accepted X, Y and Z. Its initial/reset distance
is 14 world units, with pitch clamped to 0.25..1.3 radians and distance 3..24.
Orbit/zoom do not change movement or progress.
There is no camera/terrain collision or general occlusion solver: a ridge can hide
the explorer. Orbit, zoom or R recover the view. Custom courses may need more
camera adjustment than the authored default.

## Course format and input boundary

The default `.orrt` and JSON are checked in under the new crate's `assets/` and
embedded at compile time. `Terrain::load` reads those actual canonical bytes.
For editable files, pass `--course`; a new launch reads changes while R uses the
already loaded immutable snapshot, even after source files change or disappear.

```json
{
  "version": 1,
  "title": "Terrain Trail",
  "terrain": "hill_course.orrt",
  "spawn": ["-6", "6"],
  "beacons": [["-5", "3"], ["-4", "-2"], ["1", "-5"], ["5", "-2"]],
  "finish": ["4", "-4"],
  "radius": "0.5"
}
```

This closed sample schema requires exactly four ordered beacons. Unknown,
duplicate, missing or wrongly typed fields fail. X/Z and radius are fixed-point
decimal strings, at most 32 bytes each, parsed using `FP::parse`; they are never
converted through floating point. Terrain interpolation has its own canonical
floor-rounding rule. The title is 1..48 printable ASCII bytes. The terrain name is
a non-hidden sibling `.orrt` filename of at most 64 ASCII alphanumeric/underscore/
hyphen/dot bytes; directory separators, absolute paths and traversal fail.

Admission limits:

- JSON ≤4,096 bytes; canonical terrain ≤8,192 bytes
- Terrain dimensions 2..17 on both axes; spacing 1/4..8 world units
- Every terrain vertex XYZ within inclusive ±128; this bounded Q16 profile is
  exactly representable in the existing f32 view
- Spawn, all beacons and finish must sample a non-hole, in-bounds terrain point
- XZ checkpoint radius 1/4..1 world unit, inclusive

Valid anchors do not establish connectivity. Custom terrains may separate goals
with holes; reachability/pathfinding is not computed. The original default has a
fixed keyboard-input completion regression through the same public controller.

Explicit paths must be trusted and have stable, trusted ancestry for the entire
read. Existing symlinks in the path or its ancestors, special files and oversized
files are rejected before opening; opened metadata and a capped read are checked
again. No source file is written, installed or exported. Hard-linked regular
inputs are read normally. These std path checks do not defend against hostile
same-user/root replacement of a path with a symlink/FIFO, in-place concurrent
content mutation, or adversarial writable ancestors. They are not an untrusted
filesystem sandbox. Parent traversal is refused even when it would resolve to an
otherwise safe file; symlinked system/temp roots require a trusted real path.
Windows drive-relative paths such as `C:course.json` and bare `C:` are refused.
For absolute Windows paths the drive/UNC prefix is assembled with its root before
metadata checks; the root and every real ancestor still undergo symlink checks.

## Exact movement and timing

State contains fixed-point XYZ, an ordered beacon count, finish state and u64
logical tick count. At 60 logical ticks/s, cardinal motion is 4,096 raw Q16 units
(1/16 world unit) per tick. Diagonal motion is 2,896 raw units on each axis, the
conservative integer normalization floor(4096×46341/65536); diagonal distance
does not exceed cardinal distance. Arbitrary signed input magnitudes normalize
to −1/0/+1. Opposing keys cancel; releasing W does not release a still-held Up.

Each tick applies X first, then Z from the accepted X position. These are two
axis-aligned point paths, not a straight diagonal sweep. An axis candidate is
accepted only if `Terrain::sample` returns support; Y becomes exactly that
result. Each step is shorter than one admitted cell (minimum spacing 1/4), so an
axis segment cannot enter and exit an entire intervening hole cell between two
supported endpoints. This deliberately has an X-first sliding bias. There is no
jump, gravity, slope limit, collision radius, swept character shape or headroom.

The existing a–d split `[a,c,d]`, `[a,d,b]` is reused. Interior seams belong to the
+X/+Z cell; outer maxima belong to the final cell. A hole owns boundaries under
that rule. Heights use piecewise-triangle interpolation and floor once toward
negative infinity, including negative heights. No bilinear alternate surface or
hidden terrain rebuild is used.

Checkpoint distance is XZ squared distance using i128 intermediates, sampled
after both movement axes. At most one ordered beacon advances per tick. The
finish is considered only on a later tick after beacon four. No proximity claim
is made between tick endpoints; no implicit movement occurs while finished.

Wall scheduling uses exact integer nanoseconds×60 against 1,000,000,000, not a
truncated 1/60-second duration. Each frame admits at most 250 ms, executes at most
eight logical ticks and discards excess whole-tick backlog while retaining the
fraction. A long stall slows the course rather than teleporting across holes.
Focus loss, suspension, occlusion, zero/unsupported size and restart clear held
keys, drag and time backlog. Resume does not fast-forward. Synthetic or repeated
movement key presses do not restore stale input; releases still clear bindings. R repeats do
not restart repeatedly.

## Render and dependency boundary

Default features are empty: only CPU course/controller/JSON dependencies are
required. `render` enables existing `orr_terrain_view`, `orr_render` imported-scene
and PNG support. `window` additionally opts into the already locked winit. No
new registry package/version/checksum, editor/accesskit/audio dependency or root
workspace-manifest change is needed.

The admitted immutable terrain goes through `to_static_model` and is uploaded
once per live graphics scene. Moving/restarting the explorer and changing beacon
colors rebuild only a small procedural list, not the terrain. Suspension can
destroy graphics resources; a later resume recreates that one scene. The existing
`ImportedSceneRenderer` composes the imported terrain and procedural explorer,
beacons and flag with shared depth. Separate independently clearing render passes
are not used. Legacy diffuse lighting is used; there is no new PBR/environment
lighting or performance claim.

## Validation status and gates

Source implementation and authored tests are not execution evidence. CPU,
optional-feature compilation/strict checks, actual device captures, independent
source/pixel review and native-window interaction are distinct gates. The
checked-in tests cover admission/source preservation, exact queries and movement,
negative hole corners and seams, script completion, restart/timing, real window
event-state reduction, and the production rendering path.

```sh
cargo test --locked -p orr_terrain_demo
cargo clippy --locked -p orr_terrain_demo --all-targets -- -D warnings
cargo test --locked -p orr_terrain_demo --features window
cargo clippy --locked -p orr_terrain_demo --features window --all-targets -- -D warnings
# Device fixture is opt-in and must be run explicitly with a usable adapter.
ORR_TERRAIN_CAPTURE_DIR=/fresh/unique/output/path \
  cargo test --locked -p orr_terrain_demo --features render --test render -- --ignored
```

Capture files use no-replace creation; an existing name fails rather than
overwriting earlier evidence. Without the environment variable, the fixture
checks readbacks but does not save files for review.

The authored framing test requires the full explorer and current goal, including
its numeral or flag, to keep a 16-pixel viewport margin at 960×640, 480×360 and
320×240 throughout the built-in keyboard script. This concerns the initial/reset
camera, not arbitrary user orbit/zoom or custom courses. The device fixture
retains 22 PNG/raw/metadata triples, including start and all five progress states
at both 480×360 and 320×240. Goal-specific numeral/flag and explorer pixel checks
supplement the original projection, shared-depth, hole and restart oracles.

The device fixture never turns an unavailable adapter into a passing skip. Its
original PNGs/readbacks require independent visual inspection of start, uphill,
hole, progress, summit, restart and small-window legibility. Offscreen captures
and synthetic lifecycle tests do not establish native keyboard/mouse operation.
If a native display is unavailable, that gap must stay explicit and publication
remain appropriately draft. Physical-GPU performance, broad custom-course
usability and local Windows/macOS interaction are not established by a software
adapter test. Default workspace tests do not substitute for optional-feature
execution; unchanged mandatory exact-head hosted CI remains a later merge gate.

This scope does not duplicate PR152's route builder/exporter or PR166's terrain
sculpt ownership. #94/#102 remain broader ongoing work; no full terrain, physics,
navigation, ECS rollback, network or general game-export completion is claimed.
