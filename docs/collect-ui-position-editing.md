# Collect UI visual position editing

Open a saved Collect project with the editor's `collect-ui` feature, then choose
**Preview authored screens** in the inspector.

1. Choose the screen and logical viewport you want to edit
2. Choose an explicit **Move node ID** and turn on **Move UI position**
3. Drag anywhere inside the preview canvas to move that node by the drag distance
4. Release to preview the staged position using the game's normal UI rendering
5. Choose **Apply position** to accept one undoable UI edit, or **Cancel position**
   to restore the accepted document
6. Use **Save UI document** when you want to save the accepted change

The blue outline follows the proposed position while dragging. The screen itself
updates after release, so pointer motion does not repeatedly rebuild the font or
preview. **Display 75%** scales the canvas for inspection; movement still uses the
selected viewport's logical pixels, rounded to the nearest whole pixel.

Only the selected node's offset changes. Anchors, size, text, actions, screen,
parent links, draw order and other nodes remain unchanged. Moving a container
moves its children through their existing parent-relative layout. This selection
is independent of the inspector's **UI node** selection.

Staging and cancelling leave the accepted UI, files and scene history untouched.
**Apply position** adds one **Undo UI edit** entry; saving remains a separate step.
Authored buttons cannot report actions while Move mode is enabled. Turn Move mode
off to inspect their hit regions after applying.

A drag or staged candidate is cancelled if the accepted UI changes, the project
or scene changes, the screen or canvas dimensions change, focus is lost, Escape
is pressed, the preview closes, or Play starts. Leaving the canvas cancels an
active drag. UI position editing is unavailable during Play.

Verification covers actual editor controls, staged and restored pixels, one-step
Undo, Save and reopen, scaled portrait movement, nested containers and production
button hit regions. The dedicated GPU acceptance test uses software Vulkan and
compares the preview with the unchanged production renderer. It verifies
offscreen editor rendering, not native operating-system window input.
