# GUID selection and atomic position moves

The editor keeps up to 128 distinct document GUIDs in click order, with an
explicit primary GUID. A plain click replaces the selection, Ctrl-click toggles
one GUID, and idle Escape clears it. During an active drag, Escape cancels the
gesture and retains its inspection target. The existing `selection()` and inspector refer
to the primary target. Runtime entities without a document GUID remain single
inspection targets and cannot join a batch move.

Selection resolves GUIDs against each authoritative hierarchy refresh and each
proposal's own baked hierarchy. An entity slot is never substituted for a
removed GUID. Opening or replacing a document, or reconnecting, clears the set.
The position buttons move all selected entities by one exact world unit. A
multi-selection does not start a continuous viewport drag; single-entity dragging
retains its existing transaction path.

## Admission and one request

`world.patch_batch` is an additive ERP method requiring `scene_edit`. Its params
are `label`, `expected_checksum` (exact `0x` plus 16 hexadecimal digits),
`component`, `path: "pos"`, and `patches: [{guid, value: [x, y]}]`. Components are
bound to the known game adapter: `orr_physics::Body` for PhysGame, `Position` for
Arena. This endpoint edits two-dimensional positions, not arbitrary inspector
fields. The response is a bounded `{changed, count, checksum}` summary.

The editor reads exact reflected positions from ERP and uses checked addition
of their fixed-point raw integers. It captures the document checksum before
these reads. The host rejects any intervening changed document, validates every
GUID and field/value before applying, and refuses duplicate, absent, handle-only,
oversized or unsupported targets. At most 128 patches and 64 KiB serialized
params are admitted, subject to any lower existing transport limit. These bounds
do not promise a global parser, allocator or process RSS cap.

Move controls are disabled in Play (including paused Play), replay Viewer,
proposal preview, disconnected state and an existing or pending transaction.
The controller repeats these checks and the host independently requires scene
mode, capabilities and no open transaction. A preview belongs to the editor;
the host cannot infer another client's UI preview from a position RPC.

## Atomic history and an uncertain reply

`EditorDoc::apply_atomic_batch` stages the entire E1 batch on a private copy.
A successful changed batch publishes one undo entry with its label and origin. A rejected
suffix publishes nothing: scene and frame, undo and redo, history order and
metadata, dirty state, revision and GUID allocation remain unchanged. An empty
or all-no-op batch also leaves the live history intact. This scoped correction
does not change the separate multi-request `tx.begin`/patch/commit contract.
The existing `apply_batch` retains its transaction and rollback behavior,
including conservative proposal verification invalidation after a failed batch.
Only the E1 endpoint uses the stronger isolated entry.

If the move RPC loses its acknowledgement, the editor displays an uncertain
outcome and disables further edits. It never resends the batch or assumes it was
rolled back. Reconnecting establishes a fresh identity-fenced connection and
reads the authoritative document and complete history before further edits.
Restarting a local host reopens its saved file; it cannot recover or confirm an
unsaved operation in the old host. Inspect the newly read state before issuing
a new edit.

## Validation status

Validation on shared base
`569fa578f82c440ab783a4937d0e84e223bb868b` passed on Windows MSVC Rust 1.97.1,
release, locked and offline, using an existing dependency cache. The final
five atomic-batch tests and four ERP batch tests cover a changed position prefix
followed by an invalid suffix with nonempty redo, complete history preservation,
the original redo succeeding after rejection, GUID allocation preservation and
coexistence with the legacy failed-batch revision contract. The existing
stale-verification regression also passed unchanged.

Affected edit suites passed 65 tests, remote suites passed 93, and editor suites
passed 117 visible tests with zero failures or ignored tests. The editor suites
include two controller tests, 23 model tests, six multi-selection integration
tests and 20 UI tests. They cover GUID re-resolution and admission gates, an
actual accepted RPC whose reply is discarded followed by no retry and a fresh
reconnect read, Ctrl/plain/idle-Escape selection, two-position movement and one
undo. The existing active-drag Escape cancellation regression passed with its
original assertions. A final model rerun passed 23/23 after the test-only Clippy
correction, and strict Clippy for edit/editor/remote all targets passed with
`-D warnings`.

The initial new-test compile errors, failed-proposal revision regression,
Escape regression and strict Clippy diagnostics are retained separately from
their corrected passing runs. Their assertions, timeouts and golden checksums
were not weakened. The local native-window smoke test took its explicit skip
path; its visible test success does not establish native framebuffer output.
Native multi-highlight/move/undo framebuffer acceptance must be established
by the required native fixture and its artifacts in exact-head PR CI. The
original mandatory Arena and Phys smoke assertions and timeouts remain unchanged.
PR45's client queue changes are a separate branch until integration; any final
combination validation must identify its own source and CI head.
