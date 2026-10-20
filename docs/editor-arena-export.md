# Export a saved Arena from the editor

Build the editor with the default-off `arena-project-export` feature on Linux
x86-64. This slice exports a saved Arena scene and its declared sprite bindings.
Add the separate default-off `project-ui` feature to export the existing saved
`arena-korean-v1` preset (`document: null`) and its locked Korean font. Both
`arena-project-export,project-ui` must be enabled explicitly; neither enables the
other. Tilemaps, Collect UI, other UI profiles and authored UI documents remain
refused. Other profiles retain their existing routes.

1. Save the scene and accepted sprite bindings explicitly. Resolve proposals,
   gestures, transactions and pending content work. Stop Play and managed input.
   This slice requires a local host without an ERP listener.
2. Open the **Arena export** bottom tab (or **Export guide → Open export settings**).
3. Enter the saved project root, matching already-built runtime, its expected
   lowercase SHA256, and a new output directory. Nothing is downloaded or built.
4. Read the trust warning and explicitly check the runtime trust checkbox.
   Verification executes this supplied program. A matching hash does not prove
   trust. Changing the runtime or hash clears the checkbox.
5. Click **Export saved Arena**. No file picker, popup, autofocus, autosave or
   automatic game launch follows. The underlying exporter executes bounded
   smoke verification before publication; that is not an interactive game launch.

The editor compares its authoritative saved document and referenced cached sprite
metadata/pixels against admitted project files. For the Korean preset it also
compares the declared descriptor and freshly admitted font length/SHA256 with the
exact descriptor and bytes handed to the loaded preview installation. This
attests installation input, not arbitrary later changes to egui's font context.
Missing, changed or stale installation evidence fails before starting a worker.
The font and its full package/license closure remain in the active lock; no extra
UI sidecar or export metadata is invented. See [authored project UI](authored-project-ui.md)
for the existing preset/runtime contract. Missing or stale caches fail
closed: reopen explicitly rather than expecting export to refresh them. Unapplied
form text is not automatically applied. Exact manifest, lock presence, scene and
sprite-sidecar bytes are captured, including formatting-only changes. The worker
checks those against its single admitted export snapshot before reading the
runtime or staging. Complete active-lock package checks remain in force.

Each result names its captured job/source/checksum. Edits or form changes while
it runs are not included in that export. Source disk changes cause final checks
to reject publication. There is no automatic retry or deletion of old output.
After an uncertain worker outcome or failure, use **Inspect previous destination**
before explicitly retrying. Existing outputs are never replaced.

Closing the root window while a worker is owned is refused inline, even if this
tab is hidden. The worker must be terminal and joined before the veto clears.
Clearing close status is not an override. Close again explicitly afterward;
existing unsaved-work protections still apply. Normal destruction joins the
worker; this is not cancellation or a guarantee against OS kill, crash or power
loss. The fixed 16-domain status API remains unchanged.

## Verification and limits

CPU tests exercise source mismatches, cache identity, ownership and actual egui
root close/tab events. These do not prove native GPU rendering or a delivered
player. The separate ignored acceptance test `arena_editor_export` requires
`arena-project-export,project-create`, `ORR_ARENA_EXPORT_RUNTIME` pointing to a
separately built matching Linux player, `ORR_ARENA_EXPORT_EVIDENCE_DIR` naming
a fresh absolute retained evidence directory outside source/runtime directories,
bubblewrap and a working production
capture adapter. Run it explicitly with `--ignored --exact
actual_widget_generated_edited_project_relocates_and_runs_with_sprites`.
It creates and edits a starter, saves/reopens/plays, clicks the real export widget,
relocates the bundle, hides workspace/project/runtime sources with bubblewrap,
mounts the bundle read-only, and compares movement/fire/relaunch plus production
sprite pixels. It does not treat zero-tick smoke alone as acceptance.

No new test pass is implied by this document. Record source SHA, exact commands,
results and GPU adapter separately; Linux software GPU evidence is not Windows,
native-window or physical-GPU evidence.

The acceptance directory retains command stdout/stderr, incremental stage outcomes,
producer/source hashes, source and bundle inventories, and both PNG captures even
when an assertion fails. A final receipt is written only after joined success,
source hiding, read-only write denial, replay/pixel and source-preservation checks.

## Saved Korean preset acceptance

The separate ignored `arena_editor_ui_export` target requires
`arena-project-export,project-create,project-ui`. Run
`actual_widget_saved_korean_ui_exports_relocates_and_runs` explicitly with a fresh
`ORR_ARENA_UI_EXPORT_EVIDENCE_DIR`, a separately built UI-aware
`ORR_ARENA_UI_EXPORT_RUNTIME`, and an independently built project-only
`ORR_ARENA_UI_PROJECT_ONLY_RUNTIME`. Also supply a freshly pinned sample libtest
producer as `ORR_ARENA_UI_APP_TEST_PRODUCER`; the acceptance explicitly runs its
ignored `app::project_ui_tests::relocated_bundle_real_widgets_restart_original_scene`
case inside the same isolation, rather than treating an ignored listing as a pass.
Bubblewrap isolation and a working production
capture adapter are mandatory; there is no skip-to-success fallback.

This acceptance exercises Save/reopen/Play through direct production Editor
method calls, then clicks the actual export widgets. It preserves source bytes, checks font/OFL/copyright closure, and runs the relocated player
with sources hidden and its bundle read-only. A separate isolated consumer opens
that bundle through the production admission/compositor/UI modules, clicks real
egui Play/Menu/Continue/Restart buttons, and captures title, HUD and menu pixels.
Its absent-overlay control must fail the same visible-HUD assertion. The distinct
same-length, parser/corpus-valid metadata mutation test checks installation byte
identity; it is deliberately not presented as a visible font change.

Module input coverage is not OS-window control of the exported executable. GameUi's Restart button/action alone does not prove the runtime App's simulation
reset. The separate relocated App case clicks the same buttons through the private
production dispatcher and checks original tick-zero checksum, entities, positions,
players, cleared bullets and repeated movement/fire checksums after Restart.
Existing production App dispatcher tests remain required as well. This
section describes acceptance code, not an executed pass or release claim.
