# Terrain surface appearance and delivery

This optional presentation feature replaces Navigation terrain's green checker
with one opaque image repeated once per cell, using the existing nearest sampler.
It does not change heights, holes, navigation, simulation, shaders or physics.
There are no painted layers, PBR maps, normal alignment or collision changes.

## Contract

The first profile accepts original RGB8 or fully opaque RGBA8 PNG, at most
512×512 pixels and 4 MiB encoded. Transparency, tRNS, interlace, animation and
other sample formats are refused. Stored samples are retained without gamma
conversion or resizing. Metadata admission is intentionally bounded; this is
not a general PNG decoder. See the [PNG specification](https://www.w3.org/TR/png/).

A canonical `.orrt.surface.json` sidecar embeds the RGBA image and binds the
scene-confined terrain path and stable asset identity. Height and hole edits
retain appearance. Different terrain identity, malformed sidecar or changed
operation inputs refuse explicitly. A missing optional sidecar preserves the
checker; a present invalid sidecar is never silently treated as missing.

Pending authoring separately pins current scene/terrain/source bytes and context.
Rendering caches use both terrain revision and appearance digest. Edit and Play
consume the same appearance. The original PNG is unnecessary after saving the
embedded sidecar.

## Author and deliver

Enable the default-off `terrain-surface` Editor feature. Load the local terrain,
preview the PNG, then explicitly Apply Appearance. Appearance Undo/Redo and Save
are separate from scene/terrain and ERP history. Save terrain and rebuild/save the
route when geometry changes. Terrain Open is not an appearance reload: use
Reload Appearance after external sidecar edits. File checks happen at explicit
operations/reload, not continuously. History keeps at most eight entries and
16 MiB of retained encoded appearance data. Unsaved appearance blocks closing or exporting;
failed edits must not replace the current appearance or consume its history.

Build `navigation_surface_export` and `navigation_surface_player` with the
`authored-surface-bundle,gpu` features on package `orr_navigation_view`. Invoke
`navigation_surface_export --scene SCENE --runtime PLAYER --runtime-sha256 SHA256
--trusted-runtime --output NEW_DIRECTORY`. Run the delivered
`run-navigation-surface --ticks 48 --restart-check --capture OUTSIDE/new.png`.
All paths may be absolute; the output directory and capture must not exist.
The surface
profile carries scene, terrain, embedded appearance, runtime and fixed launcher,
plus its closed integrity manifest. It requires explicit trusted-runtime input;
content hashes are integrity checks, not signatures or license verification.
The surface player renders verified manifest bytes, not an ambient sidecar.
The first capture profile is Linux x86_64, with a runtime no larger than 64 MiB.
Capture output belongs outside the closed bundle directory and cannot overwrite
an existing file.

The legacy navigation player/exporter remain separate: their V1 capability
output, four payloads, parser and bundle capture refusal are unchanged. The
Editor refuses legacy export when it would silently omit an appearance.

## Acceptance and limits

The focused checks cover strict PNG/canonical sidecar admission, stable identity,
height-edit retention, failure-atomic history, and untouched terrain/Frame data.
The end-to-end acceptance must use actual Editor-saved files, hide the original
PNG, reopen in Edit and Play, export and relocate the bundle, hide original
project/runtime inputs, and run the delivered executable from another working
directory. Captures must contain the authored pattern, route and moving agent;
full Frame and trajectory comparisons establish that appearance does not change
navigation. Legacy V1 regressions remain independent checks.

Linux offscreen software-GPU evidence does not establish native-window input,
Windows or physical-GPU support. Filesystem operations require trusted,
quiescent ancestry; snapshots are not an OS sandbox. Publication warnings after
creation are uncertain durability, not permission to overwrite or retry blindly.
