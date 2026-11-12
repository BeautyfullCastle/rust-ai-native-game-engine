# Start an Arena project with your own tiles

The optional Linux `project-create,tilemap` creator accepts a small existing
sprite package as the background tileset. Keep the bundled actor artwork and
use your own static tile regions without changing simulation or runtime formats.
This creates a new project; it does not replace packages in an existing project.

## Create and edit

Prepare a package with the existing `orr_pkg create-sprite` workflow, then run:

```sh
orr_new_arena --template arena-custom-tileset-2d-v1 \
  --output /games/new-arena --seed forest \
  --tileset-package /art/forest-tiles --tile-width 16 --tile-height 12
orr_editor --project /games/new-arena
```

Use binaries built with `project-create,tilemap`. The input package directory
must contain exactly `orr.package.json`, `sprites.json`, one PNG atlas named by
the sprite document, and `LICENSE.txt`. Its only capability is `sprite`, it has
no dependencies, and its name cannot be `sample-sprites`. At least one static
region must match both requested tile dimensions. Other valid regions/clips
remain in the package, but only matching static regions appear in the tile
palette. No new image decoder or implicit asset conversion is introduced.

The new map has 16 columns and 12 rows, starts empty, and is centered at world
origin. One pixel is one world unit, so changing tile dimensions changes the
visual map footprint. Empty cells reveal the ordinary background. This layer
has no collision or navigation meaning. The original two Arena actors and their
sprite package stay unchanged.

Choose a region in **Tile background**, paint, use **Undo tile**/**Redo tile**,
then **Save tilemap**. Close and reopen the full project, then Play. Scene Save
does not save tiles. Existing dirty, stale-source and package-lock protections
continue to apply. The creator never installs into an already-created project.

## Bounds and saved delivery

Each input file is at most 1 MiB, with a nonempty UTF-8 license at most 64 KiB.
The complete staged project retains the existing 2 MiB and 16-file limits,
including the bundled actor assets. The custom source must be an ordinary
nonsymlink directory with ordinary files. Unsupported metadata, paths, missing
or extra files, invalid images, and a palette without matching regions fail
before publication. Existing destinations are never replaced.

Creation captures source bytes and rechecks the source before publication.
Payload JSON, PNG and license bytes are preserved; installed package manifest
formatting is canonicalized by the installer. Ordinary source-change detection
is not a sandbox against hostile concurrent filesystem mutation.

Export the explicitly saved project with the existing
[Arena tilemap Linux exporter](arena-tilemap-export.md). Its verified package
closure carries both the custom tileset and bundled actor artwork. The relocated
player needs neither the original package directory nor the authoring project.
The creator adds no new runtime, editor feature or native-window support.

## Acceptance

Focused checks cover reproducible creation, strict source and dimension
admission, legacy starter preservation, no-overwrite behavior, and unchanged
simulation identity. Separate actual App checks exercise palette selection,
paint, Undo/Redo, Save and fresh reopen/Play. Delivery checks consume saved bytes
through the built exporter and relocated player, with custom tile pixel controls
and preserved actor artwork. Software/offscreen results do not establish native
window input, Windows, or hardware-GPU behavior; execution results are recorded
separately from these test contracts.
