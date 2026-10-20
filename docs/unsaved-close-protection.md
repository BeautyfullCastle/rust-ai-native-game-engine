# Unsaved work and native window close

The authoring editor refuses a native root-window close when its current inventory contains known unsaved local documents or supported staged authoring work. This guard runs after the ordinary App callback, including host/worker polling, widget edits, and existing early-return paths.

## Popup-free interaction

The status bar always reserves a fixed-width **Unsaved work** control. After a refused close it reads **Close blocked**. Refusing a close does not open a popup, select another tab, transfer focus, consume Escape, or require an acknowledgement. Normal editing, explicit saves, ERP work, and screenshot polling continue.

Click the status control or the **Unsaved work details** bottom tab to inspect the embedded read-only list. Each row identifies its existing Save, Apply, Undo, or gesture-resolution location. **Clear close status** only clears the last-close indicator. It does not save, discard, cancel a draft, or grant permission for a later exit.

There is no Save All, automatic Apply, Discard, or automatic exit. Even when all listed work has been resolved, issue a fresh native close request. Ordinary Escape keeps its existing editor/gesture behavior; it is not intercepted by the passive indicator.

## Independent authoring boundaries

- A locally owned scene uses the editor's known dirty state. A dirty remote host scene does not block closing a client because the host's document is not owned by that window.
- Retained local sprite/model/animation/irradiance bindings, tilemaps, terrain, Collect UI documents, Room camera, and Mesh authoring panels use their existing dirty or staged-state predicates when their features are enabled. A retained local sidecar still counts while attached to a remote scene or when its inspector is hidden.
- Collect UI preview position, resize, alignment, reparent, and pointer-owned staging is classified separately from the accepted document. An unapplied or unfinished row does not imply it is already saveable.
- Scene Save is not a sidecar save. Resolve and save independent documents using their own controls. Cancelling or failing Save As does not clear their protection.

## Agent-facing boundary

The list is an internal read-only inventory rendered through the existing accessible UI. This change adds no ERP endpoint, CLI command, save-all operation, or force-exit permission. `orr status --json` still reports host state/history, not all editor-sidecar state; `orr save --write` is an explicit scene write, not a sidecar save. Agents can explicitly open the embedded details through normal UI automation without first dismissing anything.

The controlled `--screenshot` completion path is separate from a native close event and remains unchanged. The Player is not governed by the authoring-window inventory.

## Limits

This is known-dirty/staged-document protection, not a universal no-loss or asynchronous shutdown guarantee. It does not cover OS force-termination, crashes, or all pending/uncertain mutation outcomes. Operation-specific scene gestures and uncertain batch completion need separate delayed-ack/lost-ack contracts and tests. Pending host reads are deliberately not a blanket close veto. No remote shared state is saved, undone, or discarded by refusing a client close.

An existing, separate limitation remains: ordinary multi-request transactions can clear a pre-existing redo branch even when their document edits are rolled back. This is covered by `crates/orr_remote/tests/erp.rs` and is not changed by this guard. The stronger complete-redo preservation contract in [multi-selection](multi-selection.md) belongs to atomic batches, not ordinary transactions. Close-protection rollback controls therefore start from a distinct saved-document fixture and still require exact file, checksum, full history, and mode preservation.

The focused tests use the real `EditorApp` callback without a GPU/native window. ERP screenshot tests exercise real request/poll/completion handling with synthetic screenshot events, not native framebuffer readback. The staged Collect UI test seeds the existing production draft state, not a pointer-render acceptance run. Linux optional-feature compilation does not establish Windows runtime behavior. Existing unrelated editor dialogs are not redesigned by this change.
