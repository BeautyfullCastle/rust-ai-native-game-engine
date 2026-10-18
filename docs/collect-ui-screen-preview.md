# Collect authored-screen preview

Build the editor with `--features collect-ui`, then open a saved Collect project
whose manifest admits an authored UI document and font. In the inspector choose
**Preview authored screens**. The existing **Collect UI document** panel remains
the authoring surface. For staged canvas movement, see
[visual position editing](collect-ui-position-editing.md).

## Preview controls

- Choose **Title**, **Playing**, **Menu**, or **Terminal** explicitly
- Choose **Landscape 640 × 360** or **Portrait 360 × 640** logical pixels
- Set sample score, result and best text; these values do not read or change the
  running game's score or progress
- Click an authored button to see its **Observed action**; Play, Menu, Restart,
  Continue and Quit are observations only
- Close the preview to release its GPU resources; reopening uses the current
  accepted document

Accepted label, button, position, size and hierarchy edits appear in the preview
without saving. **Undo UI edit** updates it as well. **Save UI document** retains
its existing meaning: save the authored UI sidecar. Preview controls never save
files or add scene history entries.

The window uses the production `CollectUi::show` layout, text, clipping and
pointer hit testing. A separate egui context, renderer and bounded offscreen
texture share the editor's existing GPU device. Resizing replaces the texture
while retaining the renderer's font atlas. Text uses the font bytes admitted
when the project was opened; the preview does not reread the package or install
a substitute font.

Pointer coordinates are translated only inside the visible preview canvas.
Leaving the canvas, focus loss, Escape, layout changes and a fresh preview
context cancel a pending press. A later release cannot revive it. Keyboard
activation is not forwarded into the canvas. Clicking an authored button does
not change the selected preview screen or dispatch a game command.

A scene path change, new scene-load epoch, absent UI panel or project replacement
invalidates the preview. Invalid accepted data is refused rather than rendered
as though it were current. A missing GPU displays a clear notice while leaving
authoring available.

## Verification

The CPU suite covers admitted-byte ownership, document refresh, bounds, actions,
font validation, scene epochs and close/reopen lifecycle. An actual `EditorApp`
widget test edits a label, undoes it, saves and reopens it without modifying the
scene or host history.

```sh
cargo test -p orr_editor --features collect-ui --lib collect_ui_preview
```

The explicit GPU acceptance test requires a software Vulkan CPU adapter and a
pre-created absolute evidence directory. It fails if either is unavailable;
being ignored in an ordinary test run is not GPU acceptance.

```sh
ORR_COLLECT_UI_PREVIEW_EVIDENCE="$PWD/evidence" \
  cargo test -p orr_editor --features collect-ui --lib \
  app::collect_ui_preview::tests::actual_editor_preview_gpu_matches_production_and_never_dispatches \
  -- --ignored --exact --nocapture
```

The test retains seven full-editor/canvas PNG pairs plus a JSON receipt:

1. Default Title in landscape
2. Unsaved label and button edits in landscape
3. The same unsaved document in portrait
4. Playing in portrait
5. Menu in portrait
6. Terminal in landscape
7. Saved and reopened Title

Each canvas is compared byte-for-byte against an independently driven production
`CollectUi` with the same document, admitted font and sample HUD. The 1800 × 1200
editor captures include full-width controls from the initial layout. Checks also
cover clipped button hit regions, pointer leave cancellation, action observation,
unchanged host checksum/history/mode/input, Undo/Save/reopen and resource release.
This is real offscreen GPU rendering of the production eframe app; it does not
claim native OS-window creation or physical mouse/keyboard injection.
