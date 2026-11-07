# Preview a model draft in the scene

In a local Yard3D or Room scene, select a body and open Models. Load the intended model asset, edit its local translation, rotation and scale, then click **Preview model draft on selected object**. Animated assets use the selected clip and explicit pose seconds. This samples one pose rather than advancing the game clock.

The viewport temporarily replaces only that body's applied model (or its ordinary body drawing). Other objects retain their models. **Cancel model draft preview** restores the applied appearance. **Assign** remains the separate binding edit; save model bindings to persist it.

Preview does not modify the simulation Frame, collider, scene file, binding document, undo history or package. It checks the same scene model admission and aggregate budgets as assignment on a cloned candidate document. The preview captures a freshly verified installed asset; viewport drawing does not reload files each frame.

Changing the selected body, scene load epoch, body pose, model form, animation pose time, binding document or admission context retires the snapshot. Leaving coherent Edit mode also retires it. Returning to an earlier value does not reactivate a retired preview: click Preview again. Closing the panel alone leaves a valid preview visible.

If an existing model-render error prevents a fresh viewport submission, the viewport may retain its last successful image while showing the error; that image is not evidence of an active draft.

External file changes are not watched continuously. Preview shows its captured verified snapshot; a new Preview or Assign verifies installed content again and refuses a stale loaded candidate.

Acceptance covers widget actions, current identity and retirement guards, per-entity static/animated render composition, unchanged document/history/files and later Assign. CPU render-list checks do not establish GPU appearance. The connected desktop is locked; native window interaction, GPU rendering and playtest video remain unverified.
