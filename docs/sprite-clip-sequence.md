# Author a sprite clip from admitted regions

With the existing `sprites` Editor feature, open a matching Arena or Collect
scene and sprite bindings, load an admitted sprite document, and select a Region
source. The explicit sequence mode starts a private one-frame clip from that
region, even if the source atlas contains no clips. Existing timing-only variants
remain a separate entry point with fixed frame order and region references.

Choose an admitted region for each appended frame. Repeated and noncontiguous
region IDs are allowed. Select a frame to remove it or move it one position up or
down; at least one frame must remain. **Reverse clip sequence** reverses all
frames while keeping each region paired with its duration and following the
selected frame to its new position. Reversing twice restores the order. A
one-frame sequence is unchanged. Edit positive integer millisecond durations
and explicitly apply the ID or duration field before preview/export. The sequence
controls and export action share a bounded scrolling area. Pending text
clears preview and blocks export until validated; invalid Apply keeps the committed
draft. Frame selection and append/remove/reorder are disabled while either field
is unapplied, so a pending hold cannot be silently discarded. Choose Loop or Once. A new noncolliding clip ID is required. The original
atlas, regions, clips, installed package and bindings are never overwritten.

Existing limits apply before draft operations and export: 4096 frames per clip,
16384 total frames, 256 clips, 128 UTF-8 bytes per clip ID, and positive `u64`
durations with checked total duration. Rejected operations preserve the draft.
Moving the first frame up or the last frame down is a no-op. An actual frame or
timing edit clears the private preview and resets playback before another Preview.

Preview uses the existing runtime's half-open frame intervals. Loop wraps at the
exact total duration; Once holds the last frame. This is a private presentation
preview, not a scene edit or simulation action. Play pauses it. Changed source,
scene-load epoch, project or admitted texture invalidates the draft, including
when the panel is hidden. Tilemap-owned atlases retain their existing restriction.

Export writes a complete new sprite JSON file and appends exactly one clip.
It revalidates the installed source and uses the unchanged typed no-replace
publisher. It never assigns a binding or applies/saves a scene. The JSON requires
the original exact PNG; it is not a standalone package. Create a new package with
the existing Rust producer and an explicitly supplied license:

```sh
orr_pkg create-sprite --png original-atlas.png --license LICENSE.txt \
  --name my-sprites --version 1.0.0 --out new-package --json new-sequence.json
```

Install and assign the new package explicitly. Existing destination refusal,
trusted-quiescent filesystem requirements and post-publication warning semantics
are unchanged from [timing variants](sprite-clip-timing.md). No drawing, atlas
editing, automatic package update, drag/drop editor, new runtime or schema is
introduced.

Focused acceptance authors a repeated, non-grid sequence through widgets, checks
independent Loop/Once boundaries and unchanged source state, then consumes the
export through the actual producer, new package installation, hidden original
paths, fresh Editor assignment/save/reopen and later-time literal pixels. Headless
software-GPU evidence is not native-window, physical-input or Windows acceptance.
The frame-reference/order/duration workflow is also used by the
[Tiled animation editor](https://doc.mapeditor.org/en/latest/manual/editing-tilesets/#tile-animation-editor);
this does not imply Tiled file-format compatibility.
