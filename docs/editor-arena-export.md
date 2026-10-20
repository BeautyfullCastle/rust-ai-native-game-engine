# Export a saved Arena from the editor

Build the editor with the default-off `arena-project-export` feature on Linux
x86-64. This slice exports a saved Arena scene and its declared sprite bindings.
It refuses declared or loaded UI and tilemaps, including builds that support
those features. Other profiles retain their existing routes.

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
metadata/pixels against admitted project files. Missing or stale caches fail
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
separately built matching Linux player, bubblewrap and a working production
capture adapter. Run it explicitly with `--ignored --exact
actual_widget_generated_edited_project_relocates_and_runs_with_sprites`.
It creates and edits a starter, saves/reopens/plays, clicks the real export widget,
relocates the bundle, hides workspace/project/runtime sources with bubblewrap,
mounts the bundle read-only, and compares movement/fire/relaunch plus production
sprite pixels. It does not treat zero-tick smoke alone as acceptance.

No new test pass is implied by this document. Record source SHA, exact commands,
results and GPU adapter separately; Linux software GPU evidence is not Windows,
native-window or physical-GPU evidence.
