# Collect UI visual size editing

Open a saved Collect project with the editor's `collect-ui` feature, then choose
**Preview authored screens** in the inspector.

1. Choose the screen and logical viewport
2. Select an explicit **Resize node ID**, then enable **Resize UI size**
3. Drag anywhere inside the canvas: horizontal movement changes width and
   vertical movement changes height
4. Release to preview the candidate using the game's normal UI renderer
5. Choose **Apply size** for one undoable UI edit, or **Cancel size** to restore
   the accepted document
6. Use **Save UI document** to persist the accepted edit

This is canvas-drag resizing, not an edge or corner handle tool. The blue outline
shows the proposed bounds while the status displays W/H and ΔW/H. Motion does
not rebuild the preview context or font; release builds one candidate. Controls
reserve fixed-height rows so status text cannot shift the canvas during a drag.
**Display 75%** changes inspection scale: 30 display pixels become 40 logical
pixels. Logical sizes round to whole pixels; values outside 1–4096, nonfinite
coordinates, and invalid candidates are rejected rather than silently clamped.

Only the selected node's size changes. Its anchor and offset remain unchanged,
and the production layout keeps its top-left independent of its own size.
Children retain their records: their anchor positions and clipping respond to
the parent's new extent. The explicit selector is separate from the inspector's
**UI node** selector. Move and Resize modes are mutually exclusive.

Staging and cancellation leave accepted UI, files and scene history untouched.
Apply adds one **Undo UI edit** entry. Saving is separate. Authored buttons never
report actions while either editing mode is enabled; disable the mode to inspect
the applied hit regions.

An active drag or released candidate is cancelled by accepted-document edits,
scene/load-epoch changes, screen/viewport/display/layout changes, focus loss
(including an explicit focus-loss event), Escape, Close, switching modes, or
Play. Leaving the canvas cancels the drag and a late release cannot revive it.
UI resizing is disabled during Play.

Actual-editor CPU tests cover independent selection and mode exclusion. The
mandatory software-Vulkan acceptance covers the ghost, unchanged motion pixels,
one release build, production pixel parity, exact cancellation, one-step Undo,
Save/reopen, expanded button hits, parent clipping and child anchors 0/500/1000,
plus portrait resizing at 75%. This is offscreen editor rendering, not native
operating-system window-input certification.
