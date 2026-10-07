# Arena sprite playback and camera following

This optional editor presentation path connects a saved Arena scene to installed
sprite assets. It does not add simulation components or change replay checksums.
Build the editor with `--features sprites`. Existing default editor builds remain
independent of the sprite and package crates.

## Author a character

1. Install the sample package into the directory containing your local scene:
   `orr_pkg install <project-directory> --path assets/sprite_demo`.
2. Open the Arena scene in the editor. Create or select a persistent scene entity.
3. Expand **Sprite bindings (view only)**. Enter a new local sidecar filename, the
   scene path relative to that sidecar, and the installed package project path
   relative to the sidecar; choose **Create bindings**.
4. Enter package `sample-sprites` and document `sprites.json`, then **Load sprite
   document**. Choose a Region, a single Clip, or **Idle / walk** with the two
   desired clip names. Set world units per pixel and **Assign sprite to selection**.
   Each entity keeps its own binding; a batch selection is one undo transaction.
5. Select exactly one authored entity and choose **Follow selected entity**.
   **Clear camera follow** removes the saved target.
6. **Save bindings**, close the sidecar, and **Open bindings** to reopen it.
   **Undo binding** and **Redo binding** cover both sprite and camera settings.
7. Start Play, choose **Take control**, and use the normal Arena movement keys.
   Moving entities use their walk clips; stationary entities use their idle clips.
   Pause, Step, rewind and Stop use the displayed host snapshot, not the UI clock.
   Managed control keeps arrow keys in the viewport; Tab, Escape and window focus
   loss release control. This small keyboard-support fix also applies without the
   optional sprites feature.

Scene Save and binding Save are separate operations. The sidecar uses an atomic
single-file replacement; this is not a joint scene/sidecar transaction. Save As
leaves the old sidecar attached to the old scene identity. Its unsaved work remains
saveable, but rendering and assignment are withheld for the new scene.

## Camera contract

Following is active during Play, including paused/stepped frames, and resolves a
persistent scene GUID through the current coherent hierarchy and full entity
handle. It snaps without smoothing and preserves zoom. A missing target holds the
last camera position and reports a diagnostic. A new entity occupying a recycled
slot does not inherit the deleted entity's binding or camera target.

Manual middle/right pan or wheel zoom suspends live following without changing the
saved target. **Resume camera follow** re-enables it. Stop restores the camera from
before Play. A new Play session starts from the saved follow setting.

## Snapshot-driven animation contract

The editor only receives drawable positions, not authoritative movement states.
Idle/walk classification therefore observes a bounded pair of displayed snapshots
per configured GUID, covering at most eight forward ticks. It never reads player
slot zero or advances using UI time.
The same paused snapshot retains its region and cursor. A transition between idle
and walk starts the new clip at its first frame.

Seek/epoch changes, backwards ticks, changed entity identities, missing positions,
and discontinuities reset the observation baseline and clip cursor to idle.
Skipped intermediate motion transitions cannot be reconstructed from positions.
An isolated arbitrary seek is not replay-exact locomotion classification; the next
eligible forward observation establishes motion again. Entire remote Stop/Start
cycles unseen between observations may be indistinguishable if their visible
identity, tick, and recorded history are identical; this is not a session event
reconstruction system. Existing single Clip bindings retain their absolute displayed-tick sampling semantics. The explicit
atlas preview is available in Edit only and is separate from scene playback.

## Compatibility and failures

Version-1 Region/Clip sidecars continue to load without being rewritten. Adding a
locomotion pair or follow target upgrades the document to version 2 in the same
undoable transaction. New sidecars are version 2. Unknown versions and v2 features
mislabeled as v1 are rejected.

Assets must come from a verified installed package. Assignment validates both
clips before committing. Failed open/reload, invalid clip assignments and failed
saves preserve prior valid authoring state; a failed reload also retains its
last-good textures and reports the error. This cache is an explicit last-good
preview, not a claim that missing or modified package files are still valid.
Use Reload assets to revalidate the installation. Orphans remain visible in
diagnostics for explicit repair rather than being reassigned or deleted.

## Boundaries and acceptance evidence

No animation graph, camera smoothing, gameplay triggers, package-manager changes,
rendering-pipeline changes, or standalone export are included. Collider-based
picking remains unchanged; sprite images do not alter collision geometry.

The `arena_sprites` tests exercise the real Arena host and EditorApp with installed
sample assets. `arena_sprites_gpu` uses the composed egui/WGPU framebuffer. Sprite
meshes are painted after the core viewport texture, so core viewport readback alone
is insufficient evidence. `ORR_REQUIRE_GPU=1` makes GPU availability mandatory;
`ORR_SPRITE_CAPTURE_DIR` optionally stores the captures. Software-GPU readback is
not native-window interaction, physical-device coverage, or Windows validation.

The bounded design follows the common explicit idle/move animation approach in
[Godot's official 2D sprite animation guide](https://docs.godotengine.org/en/stable/tutorials/2d/2d_sprite_animation.html),
while reusing this engine's existing `CameraFollow` and package contracts.
