# Mesh game session lifecycle

The Mesh game Player separates replaying a saved recording from starting a new run. **Replay** seeks through the current recorded timeline from its first tick. It preserves the recorded future and branch history; it is not an independent viewer resimulation. **Restart** reuses the current Player host and document, stops and synchronizes the current Play session, then starts a new paused session at tick zero from the in-memory authored document already admitted from the bundle. It does not create a new host, reread the scene from disk, or write to the scene file.

## Session controls

- **Cancel restart** closes the confirmation without changing the paused session or its current recording.
- **Resume** continues the current session. During recorded playback, it resumes playback from the paused cursor; after a new session is created, it explicitly starts that session.
- **Replay** restarts playback of the preserved recording from its first recorded tick. Pause and resume do not truncate that recording.
- **Restart game…** opens the confirmation **Restart the current game?**. Confirming with **Restart game** begins a new paused run at tick zero from the in-memory authored document. The prior stopped recording remains in the existing `Editor::last_stopped()` in-memory record; there is no Player history browser, and the recording is not automatically saved. The active run is replaced, so use Replay first if you need to inspect its timeline.

After Restart, explicitly resume the new run and re-enable managed keyboard goal input if needed. Keyboard goals are gameplay inputs, not scene edits: they do not save or mutate the authored file. Scene persistence remains an explicit editor save action.

## Boundaries

The lifecycle controls reuse the existing Player host, document, recorded timeline, and managed mesh-input path. Restart resets from the in-memory pre-Play authored document already loaded from the bundle; it does not reread the bundle or create a separate simulation host. The controls do not add scene fields, simulation schema, filesystem writes, physics, collision, or a claim that a run is saved as a new scene. Restart begins paused, releases managed input, and requires explicit Resume and input re-enable. Unsaved editor changes are not implicitly written to the bundle.

The opt-in hardware UI acceptance uses the existing `ORR_MESH_KEYBOARD_ACCEPTANCE`, `ORR_MESH_KEYBOARD_CAPTURE_DIR`, `ORR_MESH_KEYBOARD_EXPORT_OUTPUT`, `ORR_MESH_EXPORT_RUNTIME`, `ORR_REQUIRE_GPU=1`, and `WGPU_BACKEND=vulkan` flow. It exercises Cancel, Replay, Restart, explicit Resume/input re-enable, and a second keyboard-driven mission win using synthetic UI input. It does not claim physical OS input. Run receipts, rather than this guide, record whether a particular validation attempt passed.
