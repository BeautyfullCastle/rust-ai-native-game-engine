# Optional Arena tile background authoring (v1)

Build the Linux creator with `project-create,tilemap` and editor with `tilemap`.
Create a new project using `orr_new_arena --template arena-tilemap-2d-v1` plus
its required output and seed arguments (see CLI help). Open that project's root
with `orr_editor --project PATH`. Existing-project Attach and external atlas
imports are not supported by this version.

The starter supplies one installed atlas and an empty 16 by 12 grid. In Tile
background, enable Tile tool, select a static region thumbnail, then click or
drag over cells in the matching Edit viewport. Erase clears cells. Repeating the
same cell value does not create undo entries. Undo tile and Redo tile have a
separate, bounded 128-edit history. This is not the scene document's Undo.

Save tilemap writes the complete sidecar. Close and reopen the full project to
verify the saved presentation, then Play. Unsaved tiles must be saved or undone
before Play or closing. Scene Save does not save tile data; scene Open and Save
As do not retarget the installed tilemap. The app refuses those operations while
the map is installed. Proposal preview, replay, disconnected hosts and mismatched
scene identity cannot paint. While enabled, the tile tool reserves viewport
pointer input, including when an edit is refused, rather than moving actors.
A paint gesture must start inside the unobstructed viewport. Pressing elsewhere
and dragging in does not paint. Palette buttons expose Static region N labels.

This visual layer replaces the runtime Arena floor presentation only. Empty and
transparent texels show the normal clear background. Tiles do not create actors,
colliders or navigation meshes and do not alter simulation checksums. Regions
must match the grid's pixel tile size. Null means empty; region ID zero is valid.
The combined bound is 4096 occupied tiles plus actor bindings and eight atlases.

The palette uses SpritePanel's admitted read-only asset and egui texture cache.
The long-lived project compositor uses a separate GPU backend representation of
the same admitted assets. There is no second image decoder or egui atlas cache.
Cache-changing sprite operations are refused while the tilemap pins that cache.
Package mutation, image reimport and export are unsupported, even when a usage
query succeeds. Usage information is not permission to remove packages.

Save compares the root/scene identity, manifest, lock and exact original/saved
sidecar bytes and pinned actor-sprite sidecar before replacing the file through a
same-directory temporary file. Save also verifies actual installed package bytes
against the unchanged lock rather than trusting the decoded cache. The panel is
bound to the admitted host/scene-load epoch; a same-path host restart does not
rebind it. Scene reads retain the shared 4 MiB admission limit.
Failures preserve the existing file, document, history and saved marker. This is
one sidecar transaction, not a scene/manifest transaction, a hostile-filesystem
sandbox, or a guarantee against power-loss directory-entry loss.

## Verification scope (2026-10-20)

Local Linux validation passed 29 CPU cases (25 directly related and four existing
creator regressions), three software Vulkan cases on SwiftShader Device
(Subzero), four feature-disabled project-admission refusals and one disabled
creator-template refusal. The GPU cases cover background composition, actual
EditorApp input/Save/reopen/Play, and saved runtime composition with independent
original-texel and Frame-bytes/checksum checks. Four full egui screenshots are
visual evidence; four runtime captures have an independent pixel oracle.

Strict Clippy passed for the selected production libraries and three new sample
integration targets, including the final runtime fixture. This is not a full
workspace or editor `cfg(test)` Clippy claim. Export-enabled library compilation
was checked, but export functionality remains unsupported and was not accepted.

Original failed attempts are retained in the issue's validation history: a
missing test import, interleaved shader-cache warnings confusing the result
collector, an oracle missing the existing actor orientation mark, and a
wrong-region control whose differing texels were actor-occluded. The oracle was
corrected without changing production rendering, tolerance or excluded edges;
the final negative control explicitly checks five visible differing pixels.

The existing Linux CI job reuses a CPU25/GPU3 batch and fail-closed evidence gate,
with 17 pure-Python gate tests. Hosted validation requires llvmpipe; local
SwiftShader is not that result. Native OS input, Windows, hosted required CI,
shared development integration and main reflection are separate, unverified
stages. See [issue #258](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/258).
