# Three playable Room puzzle starters

Use these puzzles as starting points, not a connected campaign. Each original
directory contains a text grid, winning `route.json`, deliberately failing
`shortcut.json`, and descriptive `puzzle.json`. Metadata never supplies commands.

| Starter | Layout | Victory | Direct shortcut |
|---|---|---:|---:|
| `courtyard` | Enclosed courtyard with a central obstacle | 202 ticks | Fails at 122 |
| `switchback` | Two alternating barriers requiring a zigzag | 362 ticks | Fails at 202 |
| `passage` | Dividing wall with two southern passages | 242 ticks | Fails at 122 |

All use `--border --merge-walls`; only `passage` additionally uses
`--opening south:0 --opening south:6`. Original routes use semantic inputs and
require projects without authored controls. They collect the adjacent key,
release interaction while moving, then explicitly assert victory.

## Create and play

Run from the repository root with trusted production binaries on PATH. Choose
an empty working directory and nonexistent project destination. Replace the
absolute paths below. `set -C` prevents redirection from overwriting files;
nothing in this README executes automatically.

```sh
set -C
python3 -B tools/create_room_grid.py --input examples/room-grid-puzzles/courtyard/grid.txt --border --merge-walls > /absolute/new-courtyard-layout.json
orr_new_arena --template room-escape-3d-v1 --seed starter-courtyard --output /absolute/new-courtyard-project --room-layout /absolute/new-courtyard-layout.json
orr_playtest_room --project /absolute/new-courtyard-project --script examples/room-grid-puzzles/courtyard/route.json --json
orr_playtest_room --project /absolute/new-courtyard-project --script examples/room-grid-puzzles/courtyard/shortcut.json --json
```

The shortcut intentionally exits 1: assertion failure, key retained, no victory.
The winning command exits 0 and repeats with an identical report. For
`switchback`, substitute its name in paths and seed. For `passage`, substitute
its name and append the two opening options to the **grid compiler** command
only, never the creator/player.

For a schematic preview, run the same compiler options with `--format svg` and
save stdout to a new `.svg`. Alternatively paste the JSON into Room level
import, Preview/Apply, then save explicitly. Import replaces current walls;
use a fresh scene or inspect the preview. Changing geometry, rotation or
controls requires matching routes and new verification.

Creator features: `project-create,room-project`; semantic CPU player:
`room-playtest`. Python 3.11+ and Linux safe-file flags are required for the
compiler. See [grid authoring](../../docs/room-grid-authoring.md) and
[playtest](../../docs/room-playtest.md) for limits and build details.

## Optional arrow keys and Space

`controls/arrows.json` binds left/right/up/down arrows to movement and Space to
interaction. Matching supplied-key scripts live in `keyboard/<starter>/` so the
original four-file puzzle directories stay unchanged. They reproduce the same
semantic inputs and per-step frame checksums, including interaction release.

On Linux x86_64, close editors and other writers. For a newly created plain Room
project, save a **new** root-local sidecar and attach it explicitly:

```sh
set -C
cat examples/room-grid-puzzles/controls/arrows.json > /absolute/new-courtyard-project/arrows.json
orr_attach_room_controls --project /absolute/new-courtyard-project --controls arrows.json
orr_playtest_room --project /absolute/new-courtyard-project --script examples/room-grid-puzzles/keyboard/courtyard/route-keys.json --json
orr_playtest_room --project /absolute/new-courtyard-project --script examples/room-grid-puzzles/keyboard/courtyard/shortcut-keys.json --json
```

Substitute the starter name for other puzzles. The attachment CLI requires
`room-controls-attach`; key playtests require `room-playtest,room-input-actions`.
Attachment preserves the original manifest as `orr.project.before-controls.json`
and never overwrites the sidecar. Existing controls or backup cause refusal;
inspect the manifest/backup after any failure, especially an applied-but-durability-
uncertain outcome, rather than blindly retrying. Do not edit the manifest by hand
as a substitute for attachment. After attachment, the old schema-1 semantic
scripts intentionally refuse the project; use the schema-2 key scripts instead.

Key reports expose captured `controls_sha256` and resolved inputs. These are CPU
translations of supplied key identities, not hardware keyboard/window/focus tests.
The shortcut remains an intentional assertion failure.

## Acceptance and scope

Run the separate unit suites:

```sh
python3 -B -m unittest discover -s tools -p test_room_grid_puzzles.py -v
python3 -B -m unittest discover -s tools -p test_room_grid_keyboard.py -v
```

Each test module's `run_integration(binary_dir, new_receipt_dir)` takes an explicit
trusted producer directory and unused receipt directory. Helpers retain commands,
bounded outputs, hashes, repeated victories and intended failures. Keyboard
acceptance also checks exact backups, semantic/key checksum parity, refusal of
old scripts/repeated attachment and unchanged files after playtesting. Helpers
never build, download or run commands from metadata, and only create new fixtures.

These scripted CPU examples do not establish natural-language generation,
automatic solving, native/GPU rendering, audio, export or arbitrary edited-puzzle
solvability. The passage route intentionally travels outside its border. Existing
engine checks and unresolved failures remain separate.
