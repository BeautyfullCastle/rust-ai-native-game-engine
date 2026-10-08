# Bounded authored Room HUD

Optional sample feature `room-ui` adds a `room-authored-v1` presentation adapter.
The editor's separate `room-ui` feature admits the same document editor. The
original `room-project` feature and `room-escape-3d-v1` template remain UI-free.
The new `room-escape-ui-3d-v1` template installs the existing allowlisted Korean
font package alongside the Room model package; no downloads, scripts or compiler
execution are involved.

A schema-2 Room entry may declare `ui` with profile `room-authored-v1`, a distinct
root-level `document`, and a locked package/asset font reference. The authored
document retains schema 1 and its limits: 64 KiB, 32 nodes, depth 4, closed kinds,
screens, actions and geometry. Default document parse/validate remains Collect-only.
Room uses explicit profile validation and accepts only key-acquired, exit-state,
and Room-phase dynamic bindings. Collect score/phase/best cannot enter Room and
Room bindings cannot enter Collect.

The document editor uses its own bounded Undo history and atomically saves only
the admitted UI document. Its save is independent of scene, model and camera saves.
The runtime owns admitted bytes and does not reopen them on Restart. A no-UI
consumer rejects the declaration before scene or font decoding, including when
other workspace features happen to compile UI support.

Runtime screens are Title, Playing, Menu and Terminal. Play and Restart replace the
simulation with the admitted initial Frame and restore the declared camera.
Menu blocks gameplay controls but does not suspend fixed simulation ticks.
Focus/minimize retain the Room application's existing timing behavior. UI ownership
clears held gameplay/drag state; held E/R need release and a fresh press. Layout and
DPI changes invalidate pointer ownership. Native Quit exits the application;
headless capture never performs authored actions.

This feature reads RoomRun key/win state. It adds no simulation component, persistent
score, progress profile, checkpoint or game identity. Room camera remains view-only.
The export retains the active font/model package closure, notices and UI document,
then runs its existing bounded runtime smoke; a runtime without Room UI is rejected.

## Verification

The implementation must pass the Room and existing Collect/default feature matrices,
profile/schema negative tests, actual editor widget edit/Undo/Save/Reopen, runtime
widget/Frame/camera restart tests, mandatory GPU composited captures, and source-hidden
relocated read-only export tests. Focused local success is distinct from required
remote CI and development integration. Physical desktop validation is not implied by
headless app/widget harnesses or software-GPU screenshots.

Room UI editing is bound once to the admitted local scene and editor source
generation. Successful scene Open, Save As to another path, and external ERP
scene loads/save-as retire that binding through the existing camera source-change
hook, even for camera-free projects. A→B→A does not reactivate it. Failed Open
preserves the binding. Retired panels retain pending documents/history but cannot
Apply, Undo or Save; reopening the project creates a new panel. Save also checks
the exact admitted project-manifest bytes and directory identity before replacing
the UI file.
