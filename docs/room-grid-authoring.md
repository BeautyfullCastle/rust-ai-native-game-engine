# Draw a Room puzzle as text

`tools/create_room_grid.py` reads one ASCII drawing and prints an existing
[Room layout recipe](room-layout-creation.md). Python 3.11+ standard library is
sufficient. It never writes projects/files, starts the engine or installs anything.

Save this as `puzzle.txt`:

```text
.......
PK.#..E
.......
```

Run `python3 -B tools/create_room_grid.py --input puzzle.txt`. Copy JSON into
**Room level authoring → Import Room layout recipe**, then Preview/Apply using
[the editor workflow](room-layout-import.md). Import replaces all current walls;
review and save explicitly. Alternatively save JSON as a new recipe and run:

```sh
orr_new_arena --template room-escape-3d-v1 --seed grid-puzzle \
  --output /absolute/new-project --room-layout /absolute/layout.json
```

Creator features: `project-create,room-project`. Output directory must not exist.
The helper does not replace creator admission. Shell redirection can overwrite
files independently; use Save As to preserve existing recipes.

## Contract

- Exactly one `P` player, `K` key and `E` exit; `#` wall, `.` empty.
- Equal-width rows, 1–33 cells per axis; default at most 64 wall tiles.
- One world unit per cell, complete rectangle centered at X/Z zero; columns
  increase X, rows increase Z. Coordinates are exact integer/half-integer strings.
- Default walls have X/Z half-extents 0.5 and row-major order. Recipe admission
  supplies canonical Y/height. No implicit perimeter or floor.
- LF/CRLF and one optional final newline normalize identically. Spaces, tabs,
  blank rows, BOMs, other symbols, directives, includes and expressions fail.
- Input ≤4096 bytes, path ≤4096 characters; regular file, leaf symlinks refused.
  Capture requires `O_NOFOLLOW`/`O_NONBLOCK` (Linux); unsupported systems fail.
  Ancestor symlinks are allowed; hostile concurrent directory replacement is not
  protected against.
- Success exits 0 with JSON or explicit SVG only. Content/file errors exit 1,
  syntax errors exit 2, with stderr diagnostics and empty stdout.

## Transform and enclose

`--mirror-x` reflects left/right. `--rotate 0|90|180|270` rotates clockwise in the
text grid; default is 0. Mirror precedes rotation regardless of option order.
The complete rectangle transforms, including empty cells. Quarter turns swap
width/height. Movement transforms are mirror `(x,z)→(-x,z)` and 90°
`(x,z)→(-z,x)`; recorded routes are not rewritten automatically.

Add `--border` to surround the original interior with exactly one closed wall
cell layer. Interior dimensions must be ≤31 so padded dimensions remain ≤33.
Padding precedes mirror/rotation; symmetric padding preserves actor world
coordinates relative to the equivalent unbordered transformation. No doors or
openings are generated. Existing interior walls remain. A border can make a
puzzle unsolvable by blocking an exterior bypass.

There is **no implicit merge**: without `--merge-walls`, the padded drawing must
still fit 64 wall tiles. With merging, the existing 64-box/half-extent limits
apply. Original input validation remains enforced before padding.

```sh
python3 -B tools/create_room_grid.py --input puzzle.txt --border --merge-walls --format svg
```

## Merge walls

`--merge-walls` partitions the final transformed drawing: scan row-major, take
the widest uncovered wall run (≤16 cells), extend it downward across complete
uncovered rows (≤16), then repeat. Exact coverage, no overlaps or filled empty
cells. This deterministic greedy partition is not optimal; >64 output boxes is
rejected. Rotation can change the partition/count and whether it fits.

The 16-cell cap preserves engine half-extents ≤8. Three adjacent 33-cell wall
rows become three boxes instead of 99. Actor positions remain unchanged.
Merged entities/collision boundaries differ: do not assume equal replay checksums,
collision response or performance. Validate the resulting project.

## SVG and checks

`--format svg` prints a static floor-plan schematic; `--format json` is default.
Save SVG separately and open a viewer yourself. It is not recipe-import input.
Coordinates come from the same admitted JSON; P/K/E letters distinguish roles,
wall rectangles show emitted extents, +X right/+Z down. Static allowlisted markup,
no scripts/links/external resources/input filenames; ≤256 KiB, integer pixels,
local sans-serif. It shows neither collision-sized actors, height, camera,
models, lighting nor solvability, and is not an engine screenshot.

Every option occurs at most once. Unknown values/abbreviations fail. Omitted
options preserve previous JSON/SVG bytes.

Use [CPU playtest scripts](room-playtest.md) to verify gameplay. The original
unbordered example wins with interact1, release/move Z+40, X+120, Z−40,
interact1; direct X+ is blocked. This exterior detour need not work with a border.

Run `python3 -B -m unittest discover -s tools -p test_create_room_grid.py -v`.
Explicit retained-binary helpers `run_integration`, `run_transform_integration`,
`run_merge_integration`, and `run_border_integration` take binary/output directories,
retain receipts and refuse existing output directories. Border checks compare
exterior bypass success/blocking and internal wins in all eight orientations,
plus padded maximum admission. No Rust build/download, native/GPU/export or
cross-platform acceptance is implied.
