# Collect UI sibling alignment

Open a saved Collect project with `collect-ui`, then choose **Preview authored
screens**. Enable **Align siblings**, choose a stable **Alignment reference** ID,
and check one or more other nodes in **Alignment targets**. Only siblings on the
same screen and with the same immediate parent are offered. The reference stays
fixed, independently of inspector selection or document ordering.

Choose **Left**, **Center**, **Right**, **Top**, **Middle**, or **Bottom** to stage
a candidate in the production Collect UI renderer. Horizontal center is Center;
vertical center is Middle. **Apply alignment** commits all target offsets as one
undoable UI edit. **Cancel alignment** restores the accepted preview exactly.
**Save UI document** persists an applied edit; staging never saves files, changes
scene history, or edits a running host. A no-op clears the previous candidate and
adds no history entry.

Alignment changes only the relevant offset coordinate. Anchors, sizes, parents,
IDs, actions, and the reference remain unchanged. Integer offsets round to the
nearest integer, with ties away from zero; an aligned edge or midpoint can differ
by up to 0.5 logical pixel. Unrounded offsets outside the authored offset bounds
are rejected rather than clamped. Display 75% changes visual inspection scale,
not logical alignment. Nested parent clipping remains the renderer's job.

Move, Resize, and Align are mutually exclusive. Authored button actions are
suppressed while any mode is enabled. The compact fixed-height alignment strip
and bounded target popup keep canvas placement stable as selection changes.
Reference or target changes cancel a draft, as do accepted-document, scene/load
identity, screen, viewport, display-scale, or canvas-layout changes; Escape;
focus loss (including a focus-loss event); Play; or closing the preview. Alignment
is unavailable in Play mode.

## Required verification

The dedicated alignment acceptance workflow adds focused state and actual-editor
checks. The existing screen-preview, position and resize acceptance workflows
remain mandatory. All focused gates require their exact passing inventory and
reject empty, failed or ignored runs. GPU tests are explicitly selected and
required to execute; an unavailable software Vulkan adapter is a failure.

Default-feature compilation, minimal Collect UI compilation and strict Clippy
cover production and test code. Software-Vulkan acceptance uses the production
editor and renderer with synthetic input and offscreen captures; it does not
certify native operating-system window input.

### Linux acceptance result

The four checked-in acceptance workflows were executed against the same frozen
code and workflow files. Their before/after hashes matched. The focused result
was 99 distinct passing CPU tests and four explicitly executed GPU tests, with
zero failures or ignored tests in the final runs:

- Alignment state: 10; actual alignment widgets: 1
- Preview state: 18; actual preview widgets: 1
- Position state: 19; actual position widgets: 1
- Resize state: 19; actual resize widgets: 1
- Authored UI panel: 6 on Linux (the Windows workflow requires 5)
- Collect UI runtime: 12; Collect app runtime: 11
- Software-Vulkan GPU: alignment, preview, position and resize, one each

The GPU cases verify 30 production-renderer pixel comparisons: 11 alignment,
7 preview, 6 position and 6 resize. Alignment includes root staging, exact
cancellation and Apply, all six nested modes in portrait at 75% display scale,
and Save/reopen. Same-scene reopen retains the portrait settings; its canvas
capture must be exactly 360 × 640. All 11 alignment canvas/editor capture pairs
are required, alongside the receipt's renderer, history, hit-testing,
cancellation and source-preservation assertions.

Default and minimal-feature checks and strict Clippy with production and test
code passed. The new Rust files passed formatting checks. Linux software Vulkan
is the local execution scope; Windows acceptance remains a separate CI gate.

Validation first exposed a target popup that closed after one checkbox. It now
stays open while several targets are checked, and the actual-widget test requires
that behavior. Earlier test-fixture selection failures and an incorrect
landscape expectation for the portrait reopen capture were corrected before the
final runs; the earlier failure logs were retained.

Reproduce using the exact commands and inventory checks in
[alignment acceptance](../.github/workflows/collect-ui-alignment.yml),
[preview acceptance](../.github/workflows/collect-ui-screen-preview.yml),
[position acceptance](../.github/workflows/collect-ui-position.yml), and
[resize acceptance](../.github/workflows/collect-ui-resize.yml). Each workflow
retains its logs and GPU captures, including when a gate fails.
