# Assign a model to several bodies

Open local Yard3D or an admitted Room project in Edit mode and open its model bindings. Load the static or animated model, then choose its local transform and clip as usual. **Use selected model settings** can copy an existing binding into that form.

Expand **Batch model targets**, mark the named body GUIDs, then choose **Assign model to marked bodies**. Up to128 distinct bodies receive the same model, local transform and animation descriptor. This changes presentation only; each body keeps its own world pose and collision. The global 3D selection and existing single-body Assign, Remove and Fit buttons keep their previous behavior. Fit uses the primary selected collider, not each marked body separately.

The complete target list and final binding set are validated before one model-history transaction. A missing target, body-pose mismatch, aggregate limit, invalid asset, stale loaded candidate, or Room character conflict rejects the whole batch. No valid subset is silently applied. Room character animation cannot be copied to a second actor through this action.

**Undo model** restores all affected previous bindings together; Redo reapplies them. Repeating an identical assignment creates no extra Undo step. Save model bindings explicitly, then use the normal game reopen/export flow. Scene and source package files are unchanged. Target marks are temporary UI state, not saved game data; Clear model targets removes them, and opening/discarding bindings clears them. Missing marked GUIDs block assignment until cleared. Changing the current scene while the batch controls are shown resets the marks.

Acceptance uses CPU widgets, static/animated shared assets, atomic invalid/aggregate rejection, one-step Undo/Redo, saved binding reopen, Room runtime model poses and character restrictions. It does not demonstrate native-window/GPU/audio-device play. Existing Windows positive model-publication access errors remain unresolved; those checks run on Linux without retrying the denied Windows operation.
