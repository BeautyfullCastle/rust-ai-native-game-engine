# Three playable Room puzzle starters

Use these small authored puzzles as starting points, not a connected campaign.
Each directory includes a text grid, a winning `route.json`, a deliberately
failing `shortcut.json`, and descriptive `puzzle.json` metadata. The metadata
is not a program; the fixed test catalog supplies commands and options.

| Starter | Layout | Winning route | Direct shortcut |
|---|---|---:|---:|
| `courtyard` | Closed courtyard with one central obstacle | 202 ticks | Fails at 122 |
| `switchback` | Two alternating wall barriers requiring a zigzag | 362 ticks | Fails at 202 |
| `passage` | Dividing wall; two south passages permit an outside detour | 242 ticks | Fails at 122 |

All puzzles use a closed outer border and merged wall rectangles. Only
`passage` adds `--opening south:0 --opening south:6`. The routes use semantic
inputs with default movement, not physical-key events. Each begins by taking
the adjacent key, releases interaction while moving, then asserts victory.

## Create, inspect and play

Run from the repository root. Use an empty working directory for generated
files and a nonexistent project destination. `set -C` prevents shell redirection
from overwriting existing files. Replace the absolute paths below with your own.
The tools never run automatically from this README.

```sh
set -C
python3 -B tools/create_room_grid.py --input examples/room-grid-puzzles/courtyard/grid.txt --border --merge-walls > /absolute/new-courtyard-layout.json
orr_new_arena --template room-escape-3d-v1 --seed starter-courtyard --output /absolute/new-courtyard-project --room-layout /absolute/new-courtyard-layout.json
orr_playtest_room --project /absolute/new-courtyard-project --script examples/room-grid-puzzles/courtyard/route.json --json
orr_playtest_room --project /absolute/new-courtyard-project --script examples/room-grid-puzzles/courtyard/shortcut.json --json
```

The last command intentionally exits 1 with an assertion failure, key retained
and no victory. It is a negative control, not an alternative winning solution.
The winning command exits 0; run it again to obtain the same complete report.
For `switchback`, replace `courtyard` in all paths and the seed. For `passage`,
replace the name and append `--opening south:0 --opening south:6` to the **grid
compiler** command only. Do not add them to the creator or playtest commands.

To inspect the actual authored geometry before creation, run the same compiler
options with `--format svg` and save stdout to a new `.svg` file. You can also
paste the JSON into the existing Room level-authoring import preview and apply
it, then save explicitly. This replaces the current walls, so use a fresh scene
or review the preview first. These starter routes are only certified for the
unmodified grids/options. Rotation, changed controls or geometry need matching
routes and new verification.

The creator requires the `project-create,room-project` features; the CPU player
requires `room-playtest`. These examples use the existing production binaries,
without a new dependency or build command. The compiler needs Python 3.11+ and
Linux safe-file flags. See [grid authoring](../../docs/room-grid-authoring.md)
and [playtest](../../docs/room-playtest.md) for bounds and build details.

## Acceptance and limits

Run `python3 -B -m unittest discover -s tools -p test_room_grid_puzzles.py -v`.
The separate `run_integration(binary_dir, new_receipt_dir)` helper requires an
explicit directory of trusted, already-built creator/player binaries and a new
receipt directory. It checks all assets, real creation, victory, byte-identical
repeat, each failing shortcut and unchanged project files, retaining commands,
outputs and hashes. It does not build, download, run arbitrary metadata or
modify existing projects. Test-only bounded subprocesses have time/output caps.

These are scripted CPU puzzles, not natural-language generation, an automatic
solver, native keyboard/window/GPU rendering, audio or export acceptance. The
SVG is only a schematic. `passage` intentionally permits travel outside its
border; the engine admits the scene, but no universal puzzle-solvability claim
is made for edits. Existing engine checks and their unresolved failures remain
separate from these examples.
