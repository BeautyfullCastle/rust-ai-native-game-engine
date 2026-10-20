# Collect UI subtree reparent candidate

Presentation-only authoring with a pure candidate layer and an editor adapter.
The module is registered for the optional `collect-ui` feature; the adapter adds
source/container-or-Viewport selection and explicit Stage, Apply and Cancel
controls. It reuses the existing production preview and panel document-apply
path rather than introducing another renderer, schema or history system. The
pure candidate function itself performs no history, filesystem or action-dispatch
mutation.

## Editor lifecycle

1. Select a source on the active preview screen and an earlier same-screen
   container, or Viewport. Stage prepares a candidate and sends it to the
   production preview without changing the accepted document or saved file.
2. Inspect the new ancestor clipping in Preview. Apply admits the complete
   candidate through the existing panel as one edit; Cancel restores accepted
   preview content. A same-parent selection is a no-op, not an undo entry.
3. Existing Undo restores the original hierarchy. Saving remains a
   separate explicit action. Save and reopen retain the applied parent and
   adjusted offset while preserving IDs, ordering, text, actions and descendants.

The adapter binds a draft to the original document, source/target, scene/epoch,
active screen, viewport and canvas context. A changed context or Play mode
invalidates a staged edit, and Apply rejects stale snapshots instead of rebasing.
The UI wiring also cancels on focus loss, Escape, close, host/panel replacement
and switching to other geometry tools; staged reparent participates in the
existing read-only guard.
Stage/Preview/Apply/Undo/Save/reopen passed the focused actual-EditorApp test
through offscreen AccessKit and SwiftShader CPU Vulkan on 2026-10-20. Redo
remains outside this change; PR182's separate Redo/history work has not been
combined here. This does not establish native OS input/window acceptance or a
separately launched runtime process.

## Candidate contract

`propose(document, source_id, target_id, viewport)` returns `Unchanged` for the
same parent or `Changed(Document)` for a validated candidate. `None` means the
viewport root. The Stage logical viewport must be 240..=2048 on both axes.
Original input remains immutable, including every failure. Only the source
node's parent and offset can change; flat order, identifiers, anchors, sizes,
text/actions/bindings and descendants' authored geometry remain untouched.

The existing full document serializer/validator checks source and candidate,
including the 64 KiB bound, parent-before-child, same-screen containers and
root-depth-one maximum depth four. Self, descendants, forward parents, missing
IDs and malformed/cyclic inputs fail closed. Exact thousandths of a logical
pixel determine representability: fractional offsets and values outside
-4096..=4096 are rejected, never clamped or rounded. Shared production layout
then checks every selected-screen rectangle before and after, rejecting f32
arithmetic differences even when exact authored coordinates agree.

Position preservation applies only to the specified Stage viewport, not all
resolutions. Production clipping is inherited from the new ancestor chain;
visibility and pointer hit areas can change. Rect equality is not a claim of
pixel-equivalent paint, clipping or interaction. New parent validation does not
reorder nodes to make an invalid forward reference work.

## Focused verification

One table-based CPU batch covers multiple anchors, asymmetric sizes, viewport
and container parents, nested and interleaved descendants, semantic field/order
preservation, exact production rectangles, changed clipping, offset boundaries,
no-op, invalid input, candidate hierarchy-depth rejection and a one-ULP
production-f32 drift case:

`cargo test -p orr_editor --features collect-ui --test collect_ui_reparent`

The actual-app test source is
`app::collect_ui_preview::tests::reparent_tests::actual_editor_reparent_stage_cancel_apply_undo_save_reopen_play`.
It uses `ORR_COLLECT_UI_REPARENT_EVIDENCE` and passed production GPU parity
at initial, Stage-clipped, Cancel and reopen states, then an authored Play-button
pointer action on the saved `CollectUi` and a Playing-screen render. Those
assertions do not amount to a separate runtime-process or native OS input test.
The acceptance receipt confirms four production-preview parity cases, eight
retained PNGs, Stage/Cancel/Apply/Undo/Save/reopen, same-parent no-history,
visible new-parent clipping and the saved authored Play-button click.

### Recorded focused batch

Completed 2026-10-20 04:50:47 UTC (13:50:47 KST):

- Runtime: 14/14 passed, zero failed/ignored. These are 12 existing layout,
  render and hit tests plus two new public-layout API tests, not 14 new features.
- Pure candidate: 1/1 table-based contract test passed, zero failed/ignored.
  Assertions and input rows are not counted as separate tests.
- Actual EditorApp: 16/16 passed, zero failed/ignored, comprising 15 existing
  tests and one new reparent test. Six mandatory tests normally marked ignored
  for GPU execution were explicitly included, not skipped.
- Strict Clippy (`-D warnings`): sample production library and editor production
  library plus pure integration test passed. The full `cfg(test)` App/library
  unit-test harness was not checked by Clippy.
- Default-off: sample/editor library `cargo check --no-default-features` passed;
  this is not a full default-off runtime test suite.
- All 13 recorded commands exited zero, with release/locked/offline settings
  and two build jobs. Producer builds were combined and tests executed in three
  direct batches, not repeated Cargo invocations for every assertion.

The retained evidence bundle contains `RESULT.md`, `commands-results.json`,
unique-test census files, original stdout/stderr, source-inventory fences,
`capture-inventory.json`, and `reparent/reparent-acceptance.json`. The source
fence recorded all 1,901 source files unchanged during the batch. Across the
six evidence folders there are 92 retained capture/receipt files; the new
reparent fixture contributes four parity scenes, eight PNGs and one receipt.

Acceptance used the actual EditorApp via AccessKit with the SwiftShader Device
(Subzero), CPU Vulkan backend. It does not demonstrate native OS input, a native
window, a separate runtime process, Redo, hosted llvmpipe CI, shared development
integration or main integration. The original PR250 `ptrace EPERM` failure is
preserved separately; this batch's successful unit guard does not erase or
reclassify that earlier failure.
