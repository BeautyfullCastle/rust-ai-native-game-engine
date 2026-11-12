# Remove orphan model bindings

Deleting a Yard entity keeps its model metadata so scene and model undo remain independent. A recycled physics slot never inherits the deleted entity's model GUID.

In local Edit mode, open **Static model binding** and choose **Remove orphan model bindings** when an orphan count appears. This removes every binding whose persistent GUID is absent from the coherent current scene snapshot, as one model Undo step. No selection is required. It preserves live bindings, their cached models, scene bodies, installed source assets and files.

Use **Undo model** or **Redo model** for this change. A repeated cleanup with no orphan is a no-op. To restore a deleted animated entity with its model, restore both the model binding and the scene deletion using their separate histories. Choose **Save model bindings** explicitly to persist cleanup; save the scene separately when needed.

Cleanup is disabled in Play, for remote hosts, mismatched scenes, or incoherent snapshots. The complete retained binding set must pass the same admission checks as assignment, including Room's nonempty model requirement and character roles. Invalid retained assets or a cleanup that would remove Room's last required binding fails before document, cache or history mutation. It does not repair missing assets or remove live bindings.

Focused checks cover CPU widgets, GUID recycling, independent undo, animation clip preservation, saved Room reopening and CPU model poses. They do not establish native-window, GPU or audio-device play coverage. Existing Windows file-publication access errors remain unresolved; positive publication checks run on Linux.
