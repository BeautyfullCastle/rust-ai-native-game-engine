# Sprite clip timing variants

With the existing `sprites` editor feature, open a matching Arena or Collect
scene and its sprite bindings, load an admitted sprite document, and select an
existing **Clip** source. **Begin clip timing variant** takes a private copy of
that document. Region and idle/walk sources are not converted into clips.

Enter a new, noncolliding **Timing variant ID**, set each frame's positive integer
millisecond duration, and choose **Timing Loop** or **Timing Once**. Frame count,
order, and region references remain fixed, including repeated regions. The ID
is limited to 128 UTF-8 bytes and duration input to 20 decimal digits; individual
and summed durations must fit `u64`. Full document and aggregate limits still
apply after appending the new clip.

**Preview timing variant** samples the existing sprite runtime with the admitted
atlas texture. Frame intervals are half-open; Loop wraps at the exact duration
and Once holds the final frame. **Stop timing variant preview** resets to the
first frame. Entering Play pauses private preview. Scene reload, changing source
or project context, closing bindings, and replacing the cached texture discard
the draft, including when this panel is hidden. **Discard timing variant** resets
it explicitly. No scene simulation, bindings, original clips, texture cache,
Undo history, or dirty state is changed.

Set **Timing variant output path** to a new absolute `.json` path and use **Export timing
variant JSON**. Begin and export reload and compare the complete installed sprite
document and decoded atlas pixels against the displayed source. Export appends
exactly one clip, validates the complete document, serializes and reloads it, then
publishes without replacing an existing output. Success or failure retains the
draft. Existing files, symlinks, or stale installed sources are rejected. Use a trusted,
quiescent output directory: concurrent directory/staging changes are outside the
publication contract. This publisher currently requires Unix file identity; other
platforms fail closed. Post-publication cleanup/durability warnings mean output
may already exist and must be inspected rather than blindly retried.

The JSON is not a portable package by itself: it references the original exact
PNG. To package it, use the existing CLI flow with an explicitly supplied license,
name, version, and new output directory:

```sh
orr_pkg create-sprite --png original-atlas.png --license LICENSE.txt \
  --name my-sprites --version 1.0.0 --out new-package --json timing-variant.json
```

Check `orr_pkg create-sprite --help` for the installed CLI. Installing or rebinding that
package is a separate explicit workflow. Tilemap-owned atlases retain their
existing restrictions. This is timing authoring, not pixel drawing, frame
insertion, region editing, or a schema/runtime change.

Acceptance covers complete original-field preservation, repeated-region order,
exact sampling boundaries, input and aggregate limits, source/context guards,
no-replace publication, and actual editor widgets. The workflow is informed by
[Aseprite animation](https://www.aseprite.org/docs/animation/); no compatibility
with its file format or editor is implied.
