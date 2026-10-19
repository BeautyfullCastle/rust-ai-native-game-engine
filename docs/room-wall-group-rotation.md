# Room wall group quarter-turn placement

This additive extension to [Room wall groups](editor-room-wall-stamps.md) supports
0°, 90°, 180°, and 270° placement in the X/Z plane. It does not add arbitrary-angle
rotation, scaling, linked instances, or a runtime prefab dependency. The stamp
file schema and existing scene/component schema are unchanged.

## Using the controls

Open **File → Room wall groups…**, capture or load a stamp, then choose the rotation
and enter **Stamp pivot X/Z** and **Stamp offset X/Z** as fixed-point decimals.
The default is 0° with a zero pivot and offset. Positive 90° maps an offset
`(dx, dz)` to `(-dz, dx)`. Placement subtracts the pivot, rotates, adds the pivot
back, then translates by the offset. At 90° and 270°, box X/Z half-extents swap.
Y position/extent, names, static-body orientation and all other validated Actor,
Body and Collider values are preserved. The original loaded template remains
unchanged, so repeated placements never accumulate rotation or translation.

Instantiation uses the existing host transaction: all expanded walls receive
fresh GUIDs and produce one history entry. Invalid numeric fields disable
instantiation; out-of-range placement is rejected without partial scene changes
or loss of redo history. Undo/redo and ordinary project save/reopen preserve the
expanded wall geometry; reopening does not require the source stamp file.

## Mandatory additive acceptance

`tools/check-room-wall-group-rotation.py` runs real Cargo commands with `--locked`.
It rejects missing, duplicate, failed, ignored and empty test runs. Linux and
Windows CPU jobs require these exact suites:

- Existing template/schema tests: 8
- Placement arithmetic and preserved-component tests: 7
- Placement panel parsing/default tests: 2
- Existing editor transaction tests: 3
- Existing store tests: Linux 2; Windows 3
- Linux filesystem tests: 17
- Dependency boundary integration tests: 6

Default and minimal-feature library checks and strict Clippy (`-D warnings`) run
alongside the authoring-feature lint. Windows uses `windows-projects`; Linux uses
`room-authoring`. These checks retain the original wall-group workflow and tests.
A failed pre-existing regression or unavailable filesystem operation is a failed
or blocked gate, never grounds to lower counts or skip the test.

On Linux x86_64 the workflow explicitly selects Mesa software Vulkan, then lints
and executes the ignored `room_wall_stamp_rotation_ui` integration test by exact
name. This drives the production EditorApp through egui pointer/accessibility/
keyboard events, not physical OS input. Its asymmetric three-wall fixture uses
negative coordinates and a non-origin pivot. All four rotations must match exact
component geometry, fresh GUIDs and single-entry history; failed staging must
preserve redo; the loaded stamp bytes must remain unchanged.

The 90° placement must block actual gameplay movement. The same route with 0°,
180° and 270° placements must pass the barrier position, supplying wrong-rotation
negative controls. A fresh editor must reopen the saved expanded scene while the
source stamp is hidden, preserve pixels and block the route again. The gate
requires a real llvmpipe adapter, finite gameplay coordinates and all 12 expected
PNG captures with matching byte counts and SHA-256 hashes, valid IHDR checksums,
positive dimensions, and exactly 900×850 pixels for the three viewport captures. A fresh capture
directory and exactly one executed passing UI test are mandatory; receipts alone
cannot substitute for execution.

The unchanged Windows hardware-Vulkan UI acceptance remains explicitly required
before merge. The hosted Windows CPU job supplies no hardware UI proof. No generic
hosted runner is represented as completing that outstanding requirement.

## Running and interpreting evidence

From the repository root, with `PYTHONDONTWRITEBYTECODE=1` set before Python starts:

```sh
python tools/check-room-wall-group-rotation.py --self-test
python tools/check-room-wall-group-rotation.py --evidence /absolute/fresh/cpu-evidence
# Configure VK_DRIVER_FILES / VK_ICD_FILENAMES to the actual Mesa llvmpipe ICD,
# WGPU_BACKEND=vulkan, and XDG_RUNTIME_DIR to a private writable directory first.
python tools/check-room-wall-group-rotation.py --gpu --evidence /absolute/fresh/cpu-evidence
```

CPU mode requires an absent evidence directory; GPU mode creates its own absent
`linux-ui` child. Logs are written before each command and retained on failure.
GitHub Actions uploads logs, receipts and captures even when acceptance fails.
The workflow uses normal Cargo dev/test profiles. Local runs may explicitly use
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` to reduce disk usage;
such local evidence must identify those overrides and is not an assertion that
CI used the same profile. Neither setting skips assertions or tests.

The validator self-tests use synthetic text only to test rejection behavior;
they are not engine, GPU, screenshot or gameplay acceptance evidence.
