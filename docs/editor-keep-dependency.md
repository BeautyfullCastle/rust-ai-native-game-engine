# Keep an installed model dependency in the editor

Build with `package-manager` and open a saved local Room project. Add `room-character,room-lighting` when its saved presentation declares those routes. Save authoring edits and close conflicting operations first.

In **Content packages**:

1. Choose **Verify installed packages**.
2. Choose **Select keep NAME** for an indirect installed package.
3. Choose **Preview dependency keep** and review the unchanged package revision and new direct selection.
4. Choose **Keep previewed dependency**.
5. Verify again before further operations. Parent removal remains a separate preview and explicit command.

Keep retains an exact already-installed package independently of the parent that brought it in. No version, digest, asset, saved binding, character, lighting or scene is rewritten. The source-directory field is irrelevant to this installed-content action; no original package source, registry or download is consulted. Direct packages have no keep selector because they are already retained independently.

## Checked saved state

`plan_keep` and `apply_keep` wrap the existing package keep API in an opaque saved-Room observation. The plan admits all saved files through the editor-owned capability inventory and complete Room loader, pinning project identity and exact manifest/scene/models/camera/character/lighting bytes. Before lock publication, the existing exclusive writer verifies active bytes and both full locks; the wrapper rechecks the saved observation. Changed saved content requires a fresh preview. Post-publication re-admission errors retain the published selection and report the need for inspection.

The panel accepts only a matching project generation, selected name/digest and verified previous lock. Dirty/modal blockers or closing discard a queued preview. Project/name/verified-lock changes reject stale readiness. A dispatched commit is not cancelled by later selection changes: completion is attributed to its captured project. Existing package busy and whole-frame mutation fences cover keep, including worker disconnection. Successful or uncertain completion clears cached inspection and old plans/receipts; no automatic retry, rollback or deletion occurs.

These are ordinary trusted-directory checks, not hostile concurrent filesystem isolation or a joined-worker redesign. Other consumers must still enforce their own live unsaved-state guards.

## Validation scope

Focused CPU cases cover opaque plan preservation, stale saved profile/lock/writer rejection, explicit editor consent, actual panel accessibility actions, blocked/closed preview discard, stale completions, pending mutation fences and uncertain disconnection. Advanced acceptance authors the keep and separate parent removal through the panel, verifies unchanged presentation, exports, hides the original project/runtime and launches actual native headless gameplay with model poses and conditional carried light.

This is not physical OS input, native-window, GPU pixel, audio-device or video coverage. Existing Windows error5 installation/publication paths remain unverified and are not retried. Windows compilation is separate from runtime proof.

A separate CPU pointer regression waits for scroll/layout settling before clicking the actual keep buttons. The original two-frame helper clicked a moving off-view target: Select keep stayed empty before any preview worker ran. Original failed and corrected diagnostic artifacts are retained. This is simulated egui pointer input, not native OS input or a screen playtest.
