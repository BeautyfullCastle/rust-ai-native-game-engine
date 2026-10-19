# Collect UI equal-gap distribution

Open a saved Collect project with `collect-ui` and choose **Preview authored
screens**. Enable **Align siblings**, choose an **Alignment reference**, and check
at least two other siblings in **Alignment targets**. The multi-select popup stays
open while several targets are checked. The reference participates in the selected
set; it is not a fixed endpoint unless its position makes it one.

Choose **Horizontal equal gaps** or **Vertical equal gaps**. Distribution sorts
selected siblings by their leading edge on that axis, breaking ties by stable ID.
The first and last nodes stay fixed. Interior nodes move to make the spaces
between adjacent edges equal, accounting for each node's own size. Only interior
offsets on the selected axis may change. Selection order and document order do
not select the endpoints.

Offsets are calculated with checked rational arithmetic and rounded once at the
final offset, to the nearest integer with ties away from zero. Each interior
leading edge may therefore differ from its ideal position by at most half a
logical pixel, and each resulting gap may differ from the ideal gap by up to one logical pixel.
Negative available gap space, overlap after quantization, invalid selection,
arithmetic failure and out-of-bounds unrounded offsets reject the entire draft.
A no-op adds no history entry.

**Apply alignment** accepts the complete distribution as one undoable UI edit.
**Cancel alignment** restores the accepted preview. **Save UI document** persists
an applied edit. Staging does not change the accepted document, project files or
running host. The existing alignment cancellation and mutually exclusive
Move/Resize/Align behavior also applies to distribution. Portrait and 75% display
scale affect inspection, not the logical distribution equation. Parent clipping
and button hit testing use the production renderer.

## Verification contract

The [distribution acceptance workflow](../.github/workflows/collect-ui-distribution.yml)
requires all twelve focused distribution-state tests and the actual-editor
multi-checkbox control test, with exact test names and passing counts. It also
checks default and minimal Collect UI compilation and strict Clippy over
production and test code. Linux software-Vulkan acceptance must execute its GPU
test rather than silently skip it.

The GPU gate requires eight canvas/editor capture pairs and eight independent
production-renderer pixel comparisons: original, both root axes, cancellation,
Apply, both nested axes at portrait 75%, and saved-document reopening in a fresh
EditorApp. Assertions cover fixed endpoints, a moving interior reference, offset
isolation, one-step Undo, old/new button hit regions, ancestor clipping, host and
file isolation, and fresh history after reopening. The harness uses synthetic
input and offscreen rendering; it does not certify native operating-system
window input.

The original [alignment](../.github/workflows/collect-ui-alignment.yml),
[preview](../.github/workflows/collect-ui-screen-preview.yml),
[position](../.github/workflows/collect-ui-position.yml), and
[resize](../.github/workflows/collect-ui-resize.yml) workflows and their CPU/GPU
inventories remain required unchanged. Their retained artifacts distinguish
executed Linux evidence from separate Windows CI results.

## Linux acceptance result

All five workflows were executed against one frozen source/workflow tree; the
before/after hashes matched. The final runs passed 112 distinct CPU tests and
five explicitly executed software-Vulkan GPU tests, with no failures or ignored
cases in the selected gates. The distribution scope adds twelve state cases,
one actual-editor controls case and one GPU case to the original inventories.
Together the GPU tests require 38 production-renderer pixel comparisons,
including all eight distribution comparisons and their sixteen capture files.

Default and minimal-feature checks passed. Strict Clippy passed for default,
minimal Collect UI, and default-plus-Collect UI configurations, including test
code. The exact-count guard's nine self-tests and formatting checks for the new
Rust sources passed. Windows execution remains a separate CI gate. These local
results do not assert remote CI completion or publication.
