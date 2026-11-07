# Fit a model using one sampled animation pose

Model authoring can center and uniformly scale an animated model to a selected box or sphere using an explicitly chosen clip and time. It changes the local transform draft only; existing rest-pose fit buttons remain available.

Load the verified animated model in **Models**, select the desired **Animation clip** and enter **Fit pose seconds**. Select a current box or sphere collider and click **Fit sampled pose to box** or **Fit sampled pose to sphere**. Inspect the resulting local transform, then use the existing Assign, binding Undo/Redo and Save workflow to apply it.

The numeric time must be finite and lie between zero and the selected clip duration, including the endpoint. UI admission checks the entered number before conversion to f32. Invalid time or clip selection refuses fitting rather than wrapping or silently clamping. This sampling time is an authoring reference; it does not set the runtime animation clock or playback policy.

The operation samples the actual animated model, computes its deformed bounds and reuses existing fitting rules. It centers the bounds, resets local rotation to identity, applies a uniform scale and leaves 1% space. Box fitting preserves aspect proportions inside the selected half-extents. Sphere fitting conservatively encloses the sampled bounds corners inside the radius. It does not stretch axes independently.

Only the chosen pose is fitted. Other times or clips can extend outside the collider; this is not a whole-animation envelope or physics/collision guarantee. The selected collider and simulation are unchanged. Choose a representative pose for the placement you need.

Fit requires coherent editable local state, a current selected collider/body and a matching loaded animated candidate. The source package is reloaded and checked against the candidate's package digest and source hash before changing the draft. Changed installed bytes, mismatched form selection, stale colliders, Play, bad dimensions or invalid geometry refuse fitting. A failed operation retains the current transform, candidate/cache, bindings, animation descriptor, history and files.

Assign and Save remain separate. Fit alone does not modify any binding or scene, and changing the clip/time controls does not automatically fit or assign. The time field is transient authoring UI state. Existing binding persistence and freshness guards remain authoritative.

Focused tests use independent sample-geometry oracles, non-rest motion, both collider shapes, endpoint times, strict numeric errors and containment/precision limits. Actual UI acceptance covers draft-only behavior, Assign→Undo/Redo→Save/reopen, retained clip settings, deformed-point containment and unchanged source/physics data. Existing rest fitting is regression-tested.

CPU sampled geometry and UI-harness checks do not establish actual GPU appearance, native window/input/audio or play video. The connected computer remains locked, and historical Windows publication failures are not retried. Real playtest footage requires separate execution.
