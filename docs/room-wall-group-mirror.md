# Room wall group local-X mirroring

This additive extension to [quarter-turn placement](room-wall-group-rotation.md)
adds **Mirror X around pivot** to **File → Room wall groups…**. It is unchecked
by default. Existing capture, save, load, validation, replacement and instantiate
controls remain the same. The checkbox is a session placement option: it does
not change the stamp format, scene/component schema, or source template bytes.

## Placement contract

Let the source X/Z center be `(x,z)`, pivot `(px,pz)` and offset `(ox,oz)`.
Placement follows this exact order:

1. Subtract the pivot: `(dx,dz) = (x-px,z-pz)`
2. If checked, reflect local X: `(-dx,dz)`
3. Apply the selected quarter-turn: 0°, 90°, 180° or 270°
4. Restore the pivot, then add the offset

Positive 90° maps `(dx,dz)` to `(-dz,dx)`. Reflection is around the stamp's
pivot, before rotation; it does not reflect world X after rotation or local Z.
Mirroring keeps box half-extents positive. Only odd quarter-turns swap X/Z
half-extents. Names, Y centers/heights, identity static-body orientation, actor
flags, collision materials/filters and other validated fields remain unchanged.
Repeated placement always starts with the original loaded snapshot. Unchecked
placement delegates to the existing quarter-turn path, preserving its arithmetic
and exact unchanged decimal tokens.

Arithmetic uses fixed-point values and checked wide integer intermediates. Large
pivot-relative differences may cancel before the final fixed-point conversion;
final overflow, Room bounds and the serialized stamp size are still validated.
A rejected placement changes neither the scene nor its history/redo branch.
Accepted members get fresh GUIDs in one host transaction. One Undo removes the
whole group; Redo restores the same GUIDs and exact component values. Saved
expanded walls are ordinary scene entities and reopen without the source stamp.

## Independent numeric and UI acceptance

The new CPU module has 16 exact tests under
`room_wall_stamp::mirror_tests::`. It covers all eight mirror/quarter-turn
combinations, negative coordinates, an asymmetric multi-wall fixture with a
non-origin pivot, unchecked compatibility, double-reflection identity, raw
fixed-point precision, preserved fields, immutable source bytes, wide arithmetic,
exact Room boundaries, final overflow/size rejection and whole-group host
transaction rollback, Undo/Redo and existing-entity preservation. These tests
need no display/GPU. The host transaction tests still require the normal local
host connection; unavailable sockets are a failure or blocker, never a skip.

The Linux x86_64 ignored UI test is explicitly executed by exact name:
`linux_room_wall_mirror_quarterturns_history_reopen_and_collision`, in
`room_wall_stamp_mirror_ui`. It drives the production EditorApp through egui
pointer/accessibility/keyboard events. It performs actual capture, save, load,
check, mirror/rotation selection, instantiate, Undo/Redo, save and fresh reopen.
This is composed application rendering with the native renderer options; it is
not physical OS input.

Its source centers are `(2,4)`, `(4,5.5)`, `(1,7)`, with pivot `(-1,1)` and offset
`(6,2)`. A literal oracle, independent of production placement, requires:

| Checked rotation | Wall A X/Z | Wall B X/Z | Wall C X/Z |
| --- | --- | --- | --- |
| 0° | `(2,6)` | `(0,7.5)` | `(3,9)` |
| 90° | `(2,0)` | `(0.5,-2)` | `(-1,1)` |
| 180° | `(8,0)` | `(10,-1.5)` | `(7,-3)` |
| 270° | `(8,6)` | `(9.5,8)` | `(11,5)` |

Unchecked 90° instead produces `(2,6)`, `(0.5,8)`, `(-1,5)`.
Each placement runs actual gameplay along the same starting X/Z route. Checked
90° must block movement after it starts; unchecked 90° and checked 0°/180°/270°
must pass the barrier position. Checked 270° is also the explicit wrong-axis and
wrong-order negative control: around the same pivot with offset last,
`R90 Mz = Mx R90 = R270 Mx`. The fresh reopened expanded scene must block the
route again while the source stamp is hidden. The test also checks source
geometry, loaded bytes, project sidecars and installed assets remain unchanged.

The receipt requires an actual llvmpipe Vulkan adapter and 17 SHA-256/byte-bound
PNG captures: source composed/viewport, controls and gameplay for the five
placements, saved composed/viewport, fresh source-hidden composed/viewport and
fresh gameplay. The checker fully decodes PNG chunks/pixels, validates bounds,
requires nonuniform rendered content, verifies each 890×797 interior viewport
region against its composed framebuffer, and compares exact saved/reopened
viewport pixels. All three complete frames must be 1730×1009 with identical
viewport origins, logical rectangles and panel widths. The checker independently
counts more than 1,000 changed source-versus-placed viewport pixels and matches
the receipt count. The receipt also binds the compiled UI test source SHA-256. A sorted
recursive manifest binds all retained project files and the hidden stamp to
their actual bytes. A receipt or screenshot alone is not execution credit.

## Additive gates and honest limitations

`tools/room_wall_mirror_gate.py` has independent `cpu` and `gpu` routes. The new
workflow requires CPU on Linux and Windows plus Linux software-Vulkan UI, and
its aggregate fails if any matrix route is failed, skipped or cancelled.
The checker SHA-pins the existing wall-group/quarter-turn gates and workflows,
prior template/placement/store test sources, and existing Linux/Windows UI
sources. It does not edit, disable or replace them.

The CPU route also executes the unchanged quarter-turn CPU gate, retaining:

- Template/schema: 8 tests
- Original quarter-turn placement: 7 tests
- Original placement panel: 2 tests
- Original editor transactions: 3 tests
- Store: 2 Linux / 3 Windows tests
- Linux filesystem: all 17 tests, including Unix-socket rejection
- Dependency boundaries: 6 tests
- Default/minimal-feature checks and strict Clippy with warnings denied

An existing Linux socket restriction stays a failed or blocked prerequisite.
A passing mirror subset or Linux UI run does not resolve that failure. Existing
Windows hardware-Vulkan UI acceptance is separately mandatory before merge;
these hosted CPU/Linux-software routes do not supply it. Every mirror receipt
keeps that Windows coverage **NOT RUN** and does not certify merge eligibility.
No generated receipt claims a merge, physical input, or hardware GPU execution.

## Running the checker

From a clean, committed checkout, with a fresh absolute evidence destination
outside the checkout:

```sh
export PYTHONDONTWRITEBYTECODE=1
unset PYTHONOPTIMIZE
python -B tools/test_room_wall_mirror_gate.py
python -B tools/room_wall_mirror_gate.py cpu --evidence /tmp/room-mirror-cpu-unique

# Optional explicit locally configured Lavapipe manifest; the two variables
# must identify the same single absolute manifest when both are present.
export VK_DRIVER_FILES=/absolute/local-vulkan/lvp_icd.x86_64.json
export VK_ICD_FILENAMES="$VK_DRIVER_FILES"
python -B tools/room_wall_mirror_gate.py gpu --evidence /tmp/room-mirror-gpu-unique
```

A configured manifest may live outside the system ICD directory. If neither
variable is set, the checker requires exactly one system `lvp_icd*.json` under
`/usr/share/vulkan/icd.d`. Invalid or conflicting explicit settings fail instead
of falling back. The checker selects Vulkan, creates a private runtime directory,
records the manifest hash, then checks the actual adapter returned by the test.

Each route creates a new private Cargo target, builds with `--locked`, retains
the freshly compiled test executable, and binds its package/source/target,
feature closure, profile, hash and byte count to the checkout HEAD, tree and full
source inventory. It runs the retained executable, checks its actual listed and
ignored inventories, and rejects empty, missing, duplicate, ignored, failing or
unexpected executed cases. Unrelated library tests filtered by the exact mirror
module are counted explicitly. No historical binary supplies current evidence.

Source paths, bytes and modes are fenced around every bounded subprocess and
again on completion. Ignored/untracked files outside `target`, including Python
bytecode caches, fail admission. Git worktree metadata is not source. Evidence
and retained producers are never overwritten; failure logs and original failures
remain recorded. Optimized Python is rejected before assertion-based helpers
are imported. The 29 independent Python adversarial fixtures validate the
checker only and are not Rust, rendering or gameplay evidence.

The workflow explicitly sets `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0` to limit debug-symbol disk usage while retaining
normal test optimization and debug assertions. Local overrides are recorded in
the gate receipt. All receipts, screenshots, retained executables and partial
failure logs are uploaded even when a gate fails. A successful local subset must
still be distinguished from a completed clean-checkout gate and required CI.
