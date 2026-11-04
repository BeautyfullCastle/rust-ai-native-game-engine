# Replace all matching tile cells

Enable **Tile tool**, select a static atlas region or **Erase**, then select
**Replace all** and click a seed cell in the matching Edit viewport. Every cell
with the seed's exact value is replaced, including disconnected islands and
cells touching only diagonally. Empty cells are distinct from region zero.
**Fill** still changes only the four-connected area.

Replace all runs once per primary press. Holding, dragging into the map, or
moving over another target while held does not repeat it. Invalid, obscured,
unfocused or ambiguous presses do not apply. Changing mode cancels an unfinished
gesture. A successful replacement is one tile Undo/Redo entry; selecting the
existing value is a no-op and retains redo. Atlas errors and combined tile/actor
budget failures leave the document, history and saved marker untouched.

The operation scans at most 4096 cells and uses the existing 4096 combined
occupied-cell/actor limit. Existing Edit/admission/scene guards, pinned atlas,
Save conflict checks and explicit Save behavior remain in force. It changes no
simulation state, schema, package format, dependencies or runtime renderer.

## Verification contract

CPU tests cover disconnected literal results, empty versus region zero, erase,
4096-cell bounds, invalid coordinates/regions, wrong-size atlas regions, source
immutability, grouped history, budget overflow and actual installed-panel egui
press/hold/coalesced-event dispatch. Source files remain unchanged until Save.

The explicitly ignored actual EditorApp acceptance
`actual_cli_replace_save_reopen_play_literal_cells` uses a CLI-created project,
AccessKit buttons and egui input, grouped Undo/Redo, Erase, no-op redo retention,
Save, fresh reopen and Play. It retains screenshots, literal cell arrays,
simulation Frame parity and a 10,752-pixel original-atlas runtime oracle with a
visible wrong-region control. Run with `ORR_REQUIRE_GPU=1`, a verified creator at
`ORR_NEW_ARENA_BIN`, and an existing absolute directory at
`ORR_TILEMAP_REPLACE_EVIDENCE`. Set `LIBGL_ALWAYS_SOFTWARE=1 EGL_PLATFORM=surfaceless WGPU_BACKEND=gl`.
The test requires a CPU/GL adapter. This is software GL, not native OS input.

Authored tests are not execution receipts. Linux-local required checks,
independent review and actual App execution must be recorded for the exact
candidate before publication. Windows, hardware GPU, browser and export are
not established by these tests.
