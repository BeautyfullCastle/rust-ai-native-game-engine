# Optional Arena tile background authoring (v1)

Build the Linux creator with `project-create,tilemap` and editor with `tilemap`.
Create a new project using `orr_new_arena --template arena-tilemap-2d-v1` plus
its required output and seed arguments (see CLI help). Open that project's root
with `orr_editor --project PATH`. Existing-project Attach and external atlas
imports are not supported by this version.

The starter supplies one installed atlas and an empty 16 by 12 grid. In Tile
background, enable Tile tool and select a static region thumbnail. Paint (the
default mode) changes cells by clicking or dragging in the matching Edit
viewport. Fill changes the four-connected area matching the clicked cell's
exact value with one click. Diagonals are not connected. Choose Erase to clear
the painted cell or filled area; empty cells and region ID zero remain distinct.
Fill runs only on the initial unobstructed viewport press, never repeatedly
while holding or dragging. Changing Paint/Fill mode ends an active stroke.

A complete fill is one Undo tile / Redo tile entry in the separate, bounded
128-edit history, not the scene document's Undo. Filling with the same value is
a no-op and preserves redo and the saved marker. Traversal is iterative and
bounded by the admitted grid's 4096 cells. The complete candidate is validated
against the installed atlas and the combined tile/actor budget before committing;
a failure leaves the document, history and saved marker unchanged.

Width and Height are draft whole-number inputs (1 through 64). Only the explicit
Resize button applies them; typing or pressing Enter does not change the map.
Resize keeps the grid origin and retained cell coordinates fixed, including their
world placement, tile size and scale. New cells are empty. Shrinking is refused
if any occupied cell would be removed: explicitly Erase those cells first.
A resize is one tile-history entry. Same-size Resize is a no-op preserving redo
and the saved marker. Undo/Redo resets the dimension drafts to the restored map;
Resize and Undo/Redo also end any active paint stroke. Resize uses the same Edit,
scene/epoch and atlas/combined-budget checks as other tile editing, and Save
retains the existing source-stamp checks. No new sidecar format is introduced.

Save tilemap writes the complete sidecar. Close and reopen the full project to
verify the saved presentation, then Play. Unsaved tiles must be saved or undone
before Play or closing. Scene Save does not save tile data; scene Open and Save
As do not retarget the installed tilemap. The app refuses those operations while
the map is installed. Proposal preview, replay, disconnected hosts and mismatched
scene identity cannot paint, fill or resize. While enabled, the tile tool reserves viewport
pointer input, including when an edit is refused, rather than moving actors.
A paint or fill gesture must start inside the unobstructed viewport. Pressing elsewhere
and dragging in does not paint. Palette buttons expose Static region N labels; mode buttons expose Paint and Fill labels.

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

## Original paint verification scope (2026-10-20)

The following records the pre-fill baseline, not acceptance of the Fill addition.

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

At the original Paint acceptance, the Linux CI job reused a CPU25/GPU3 batch and fail-closed evidence gate,
with 17 pure-Python gate tests. Hosted validation requires llvmpipe; local
SwiftShader is not that result. Native OS input, Windows, hosted required CI,
shared development integration and main reflection are separate, unverified
stages. See [issue #258](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/258).

## Fill local verification scope (2026-10-20)

Local Linux validation passed 34 CPU cases: the 25 existing gate cases, five new
Fill cases (three pure-candidate and two panel/history cases), and four retained
creator regressions. Four software Vulkan cases passed on SwiftShader Device
(Subzero): the three existing cases and the actual EditorApp Fill sequence.
That sequence covers outside-origin refusal, one-shot held/drag behavior,
grouped Undo/Redo, Save, fresh reopen and Play with literal saved-cell checks.

The Fill evidence contains ten captures and 43 artifacts. The saved-runtime and
wrong-region runtime captures each pass an independent original-texel oracle
checking 10,752 pixels; the wrong-region control has 37 visibly differing pixels.
Actual Frame-byte parity is checked for authoring and the App/saved-runtime Play
consumers, including the wrong-region control. Whole-app screenshots are visual
evidence, not a substitute for those pixel and Frame assertions.

Strict Clippy passed for the selected production editor library with tilemap
and project-create enabled. This is not editor `cfg(test)` or full-workspace
Clippy acceptance. The selected editor library also compiled with
`--no-default-features`; this Fill validation did not rerun feature-disabled CLI
or runtime refusal tests. Two original App-test compile errors and a pre-launch
collector-name collision are retained in the validation history, not erased by
the successful retry.

At the Fill acceptance, the existing tilemap gate specified CPU30/GPU4 and
passed 25 pure-Python validator tests, without changing a workflow or creating a
new job. Local
SwiftShader results are not hosted required-CI results: that gate requires
llvmpipe. Native OS input, Windows and exported-player behavior remain outside
this acceptance; export remains unsupported. See
[issue #261](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/261).

## Resize local verification scope (2026-10-20)

The Paint and Fill sections above record their historical acceptance runs. The
Resize run passed 39 local Linux CPU cases: all 30 prior mandatory cases, five
new Resize cases (three pure-candidate and two panel/history cases), and four
retained creator regressions. Five software Vulkan cases passed on SwiftShader:
all four prior cases plus the actual EditorApp Resize sequence.

The Resize sequence exercises the accessible Width/Height drafts and explicit
Resize button, painting a newly added cell, grouped Undo/Redo, same-size no-op,
occupied-crop refusal, explicit Erase followed by trim, Save, fresh reopen and
Play. Literal cell arrays, retained world positions, history/saved markers and
source/scene/lock preservation are checked independently of screenshots.

Resize evidence contains exactly 68 artifacts and 16 captures: 14 whole-App
captures and two runtime captures. The saved-runtime and deliberately wrong-stride
runtime captures each pass an independent original-texel oracle checking 3,840
pixels; the wrong-stride control differs visibly from the correct map at 91
pixels.
Actual Frame-byte parity is checked against the bare simulation control. App
screenshots are visual evidence, not a replacement for pixel or Frame assertions.

Strict Clippy passed for the selected production editor library both with
`tilemap,project-create` and with default features disabled. This does not claim
editor `cfg(test)`, full-workspace Clippy or feature-disabled runtime/CLI refusal
acceptance. No failures or retries
occurred in this Resize validation run; the earlier Paint/Fill failures recorded
above remain part of their historical validation records.

The existing tilemap gate specifies CPU35/GPU5. Its 35 pure-Python validator tests
and the applied gate against the actual local evidence passed, without adding or
changing a workflow or job. Local SwiftShader validation is not hosted required
CI, whose gate requires llvmpipe. Native OS input, Windows, hosted required CI and
exported-player behavior remain outside this acceptance; export remains
unsupported. See
[issue #263](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/263).
