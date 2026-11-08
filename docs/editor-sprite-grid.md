# Create a sprite package from a PNG grid

Build the editor with the default-off `sprite-grid-authoring` feature. Open a matching local sprite-bindings sidecar in Edit mode and expand **Create sprite package from PNG grid** in the Sprite panel. This creator is unavailable while the tilemap owns the shared atlas set.

Choose a PNG and explicitly inspect it. Enter cell width and height, a clip name, loop/once playback and a positive integer duration in milliseconds per frame. Supply a license text file, package name/version and a new destination, then acknowledge redistribution rights and create the package. Inspection fully decodes the PNG and records its dimensions and SHA-256. Creation checks the actual captured PNG against that hash before publication; a changed image requires inspection again.

The image dimensions must divide exactly by the cell dimensions. Up to 256 cells become row-major regions, starting at the top-left pixel with ID 0. One clip plays every region in that order. The creator does not trim transparent pixels, skip cells, handle gutters, pack irregular rectangles or infer frame timing. PNG/document resource limits and validation reuse the existing sprite loader and package authoring API; clip duration must not overflow.

The output contains `sprites.json`, byte-identical `atlas.png`, license text and the standard package manifest. It is a new source package and is not installed or assigned automatically. Install explicitly with the existing package tool, for example:

```
orr_pkg install /path/to/project --path /path/to/new-package
```

Then use the existing Sprite panel's **Load sprite document** with the package name and `sprites.json`, choose the new clip and assign it to selected actors. Binding Undo/Redo and **Save bindings** remain separate from scene Save. Reopening and playback load the installed copy, so original authoring PNG/license/package folders are not runtime dependencies.

Input or project-context changes revoke consent; stale inspection/completion results are not adopted. An already dispatched creation may still finish, and its output location is reported and retained. The existing publisher claims a new directory and writes visible files; publication is not crash-atomic. On failure, partial output may remain. Directory existence is not a completion signal: inspect it manually, and do not retry, overwrite or clean it up automatically. New editor destinations inside an `.orr` store are rejected. Creation does not install packages or alter locks, scene state, bindings or the admitted renderer cache.

Focused acceptance covers grid rectangles, clip boundaries/looping/once playback, exact PNG bytes, stale same-sized PNG rejection, partial-output preservation, real creator widgets and installed source-hidden sprite assignment/history/Save/reopen/CPU use. Windows execution, physical GPU/window rendering and actual play/video remain unverified by Linux CPU checks.
