# Draw a Room puzzle as text

`tools/create_room_grid.py` turns a small ASCII drawing into the existing
[Room layout recipe](room-layout-creation.md). Python 3.11+ and its standard
library are enough. The helper reads one local file and prints JSON. It does
not write a recipe, modify a project, start the engine or install anything.

Save this drawing as `puzzle.txt`:

```text
.......
PK.#..E
.......
```

Run `python3 -B tools/create_room_grid.py --input puzzle.txt`. Copy the JSON
into **Room level authoring → Import Room layout recipe**, then Preview and
Apply using the [existing editor workflow](room-layout-import.md). Import
replaces all current walls, so review the preview and save explicitly.

Alternatively save the printed JSON as a new `layout.json` file and create a
new project:

```sh
orr_new_arena --template room-escape-3d-v1 --seed grid-puzzle \
  --output /absolute/new-project --room-layout /absolute/layout.json
```

The creator needs `project-create,room-project`. Its output directory must not
exist. The grid helper does not launch it or substitute for its scene admission.
Shell redirection writes files independently of the helper and can overwrite
them; use your editor's Save As when keeping an existing recipe.

## Rotate or mirror a puzzle

Add `--rotate 90`, `--rotate 180` or `--rotate 270` to turn the drawing clockwise
as viewed in the text grid (rows increase Z). `--rotate 0` is the default.
Add `--mirror-x` to reflect left/right. When both are supplied, mirroring happens
first, then rotation, regardless of command-line option order. Each option may
occur at most once; other angles, negative values and abbreviations are refused.

For example, this prints a reflected and rotated recipe:

```sh
python3 -B tools/create_room_grid.py --input puzzle.txt --mirror-x --rotate 90
```
 The original text file and
existing project remain untouched. Use the same Preview/Apply or creator flow.
The whole rectangle is transformed, including empty cells; quarter turns swap
its width and height. Actor roles remain unchanged and walls are emitted in the
transformed drawing's row-major order. A 90-degree turn maps movement `(x,z)` to
`(-z,x)`; a mirror maps it to `(-x,z)`. Existing recorded routes are not rewritten,
so rotate/reflect their movement inputs too before replaying. Exact coordinates,
bounds and admission rules remain unchanged. Without options, JSON is unchanged.

## Drawing contract

- `P` is the player, `K` the key, `E` the exit. Include each exactly once.
- `#` creates one wall box; `.` is empty. No implicit perimeter or floor is added.
- Rows have equal width, 1–33 columns and 1–33 rows. By default, at most 64 `#` tiles are allowed; explicit wall merging below instead limits the emitted boxes.
- Tiles are one world unit apart. The complete rectangle, including empty tiles,
  is centered at X/Z zero. Columns increase X; rows increase Z. Even dimensions
  give half-integer centers. Coordinates use exact decimal strings, never floats.
- Wall half-extents are X/Z 0.5. The existing recipe supplies canonical Y/height.
  Walls appear in row-major order. LF and CRLF, with or without one final newline,
  produce identical JSON. Spaces, tabs, blank rows, BOMs and other symbols fail.

The input is at most 4096 bytes, captured once from a regular file without a
leaf symlink. File capture requires `O_NOFOLLOW` and `O_NONBLOCK`, available on
Linux; unsupported platforms fail explicitly. Directory ancestors may be
symlinks. Input paths are at most 4096 characters. This is a local file contract,
not protection against hostile concurrent directory replacement.

By default, success prints only schema-1 JSON and exits 0. Explicit SVG output is
described below. Invalid content or file errors
exit 1 with a diagnostic on stderr and no recipe on stdout. Invalid command
syntax exits 2. There are no map directives, includes, expressions or paths.

## Check the result

For the example, collect the nearby key, release interaction and move Z+ for
40 ticks, X+ for 120, Z− for 40, then interact for one tick. This route wins;
moving directly X+ is blocked. Use the [playtest script format](room-playtest.md).
Neither a valid grid nor scene admission proves a puzzle is solvable.

Run `python3 -B -m unittest discover -s tools -p test_create_room_grid.py -v`.
The separate `run_integration(binary_dir, new_receipt_dir)` test entrypoint uses
explicit trusted, already-built creator/playtest executables and retains its
fixtures and reports. Optional keyword arguments `rotate` and `mirror_x` exercise transformed drawings
and correspondingly transformed route inputs. It checks wall blocking, a winning detour, repeatability,
unchanged projects and boundary-size admission. It does not build Rust or prove
native input, window/GPU rendering, export or cross-platform behavior.

## Preview the floor plan

Add `--format svg` to print a bounded static SVG schematic instead of JSON:

```sh
python3 -B tools/create_room_grid.py --input puzzle.txt --format svg
python3 -B tools/create_room_grid.py --input puzzle.txt --rotate 90 --mirror-x --format svg
```

Save the printed SVG as a new `.svg` file in your editor and open it in an SVG
viewer. Shell redirection is independent of the helper and may overwrite files.
The helper still only reads the input and writes stdout; it does not open a
viewer, save files, launch a game or change a project. `--format json` is explicit
JSON output and omitting `--format` preserves the original JSON bytes. The format
option may occur once; unknown formats and duplicate options fail before input
capture. SVG is not an input format for Room recipe import.

The diagram shows P (player), K (key), E (exit) and dark wall squares on a one-unit
X/Z grid. X increases right, Z down. Mirror and rotation apply exactly as in JSON:
the actor and wall coordinates are read from that same admitted recipe. Empty
edge cells remain visible, including rectangular drawings after quarter turns.
The letters distinguish roles without relying only on color. The drawing is
static SVG with no scripts, links, external images/fonts or input filenames.
Output is bounded to 256 KiB, with integer pixel coordinates and a fixed cell
size. Generic local sans-serif rendering can vary by viewer.

This is a schematic, not an engine screenshot. Actor markers are labels, not
collision sizes. It does not show wall height, camera view, model packages,
lighting, runtime movement or whether the puzzle can be solved. Use the existing
editor and CPU playtest for gameplay checks; native/GPU acceptance is separate.

## Merge adjacent wall tiles

Add `--merge-walls` to JSON or SVG output when authoring long walls or large
solid regions. The default remains one box per `#` tile, byte-for-byte unchanged.
With merging, more than 64 wall tiles are accepted only when the resulting
partition has at most 64 boxes. All other input bounds and creator admission
remain enforced. For example, three adjacent 33-tile wall rows produce three
boxes rather than 99, because each box is at most 16 cells per axis.

The helper validates input, mirrors and rotates if requested, then partitions
the transformed drawing. It scans row-major for the first uncovered `#`, takes
the widest contiguous uncovered run (at most 16 cells), and extends downward
while that entire width is still uncovered `#` (at most 16 rows). It marks that
rectangle covered and continues. This deterministic greedy partition is not a
minimum-box solver. If it produces more than 64 boxes, it refuses the drawing;
it never silently fills empty cells or drops walls. Rotation can change the
partition and box count, so one orientation can be rejected while another fits.

Each rectangle covers exactly its wall tiles without overlap. Centers and half
extents use exact integer/half-integer decimal strings. The 16-cell cap preserves
the existing Room wall half-extent limit of 8. Actor positions do not change.
SVG rectangles show the emitted geometry; grid lines behind a large wall are
covered. No project is written by this option.

Merged boxes have different entity identities and collision boundaries from
individual tiles. Do not reuse old replay checksums or assume identical collision
response/performance. Validate the new project with the creator and playtest.
Focused retained-producer tests exercise a two-cell barrier, a blocked direct
route, a winning detour and unchanged no-wall control in all eight orientations,
plus 99-cell admission and over-64-box refusal. They are CPU checks, not native
window/GPU or exported-player validation. `run_merge_integration(binary_dir,
new_receipt_dir)` retains these explicit production checks separately.
