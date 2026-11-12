# Reusable tile patterns

With the optional `tilemap` Editor feature, use Copy to capture a rectangle,
including empty erasing cells. Open **Reusable pattern files**, enter an absolute
`.json` path, then choose **Save pattern**. The destination must not exist.
Save creates a standalone layout file, not a package or a tilemap save.

After reopening a project, enter that file and choose **Load pattern**. Loading
only replaces the private copied pattern. Choose Stamp and click its destination
to apply it, then explicitly Save tilemap. Existing one-entry Undo/Redo, whole
footprint refusal and the combined tile-plus-actor budget still apply. A changed
scene, atlas or geometry expires the loaded clipboard; explicitly Load again.

Patterns can be reused across maps with different dimensions, origins and world
scales. Both maps must already have the same sprite package/document, exact
installed package digest, canonical atlas document and tile size. A changed PNG
with unchanged sprite JSON is not compatible. The file never installs artwork,
selects a path to open, changes a package lock, or modifies runtime schemas.
Layouts include at most 4096 cells in a 64 by 64 bounding rectangle. Region zero
is a real tile; null cells erase. Loading a layout larger than the destination
map is allowed, but stamping still requires the entire layout to fit.

Save and Load require an idle matching Edit viewport and revalidate installed
assets and project source pins. Failed Load retains the previous copied pattern
and history. Save/Load do not dirty the map, alter scene state or apply a stamp.
The existing saved tilemap renderer and exporters consume the resulting cells;
the pattern source file is not required by a saved game.

Input is bounded before parsing. Unknown and duplicate fields, invalid versions,
dimensions, cell counts, regions and mismatched artwork are refused. Paths must
be explicit absolute JSON paths without traversal or symlink ancestry. Stable
file-identity support is required; unsupported platforms fail closed. These
checks require trusted, quiescent directories and are not an OS sandbox against
hostile concurrent filesystem mutation.

Publication reuses the sprite JSON publisher's exclusive staging, exact readback,
sync and hard-link no-replace contract. Existing files are never overwritten.
Once the final link exists, subsequent failures are reported as warnings: output
exists or may exist, so inspect it instead of blindly retrying. A successful
pattern save does not mean the tilemap or scene was saved.

Focused acceptance covers strict format and filesystem admission, installed
project guards, real widget Copy/Save/Load/Stamp/Undo/Redo/Save/reopen, and the
saved runtime's literal pixels and authoritative Frame parity. Execution receipts
are reported separately. Offscreen software-GPU tests do not establish native
window input, Windows support or hardware-GPU acceptance.
