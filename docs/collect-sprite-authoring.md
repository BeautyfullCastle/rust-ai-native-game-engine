# CollectDodge sprite authoring (bounded optional slice)

Priority: close the authored 2D collect/dodge project's primitive-only presentation gap in #94/#98. Depends on the admitted Collect project, verified package manager, existing sprite document/PNG decoder and progress identity slice. This is not full three-game acceptance.

## Boundary

`orr_sample/collect-sprites` explicitly enables the standalone/export consumer. Its default is off. The editor opts in through its own `sprites,collect-dodge` combination; Cargo unification of sample sprite dependencies alone never enables editor admission. `PreparedProject::open_with_presentation` requires explicit sprite and progress support. Old `open` / `open_with_progress` reject declared sprites. UI declarations remain unsupported.

The schema-2/3 entry's optional `sprites` file reuses the bounded v1/v2/v3 sidecar; v3 optionally adds [per-binding orientation](sprite-orientation.md). Every binding and camera target must be a persistent Collect Actor GUID in the admitted scene. The sidecar scene and project must resolve to the exact entry/root. Package capability/hash/closure and every active/inactive clip are checked before any host/GPU starts. Installed atlas bytes are owned after admission; no runtime file reload occurs. No project identity is minted, no open/export writes occur, and cosmetic changes leave the Frame, collision squares, checksums and semantic score digest unchanged.

## Authoring and playback

Open with `orr_editor --collect-project DIR` built with `sprites,collect-dodge`. The existing Sprite bindings panel supports independent per-actor Region, Clip and idle/walk assignment, scale, optional follow target, separate sidecar undo/redo/save and reopen. Scene Save does not save the sidecar. The assigned sprite overlays the unchanged square picking/collision proxy; unbound actors keep primitive rendering, collected actors disappear from both routes.

Region and plain clips use the authoritative run's elapsed ticks at the fixed 60 Hz, so Pause holds, Seek restores phase, terminal gameplay holds phase, and a fresh Space restart returns to the first clip frame. Locomotion retains the existing bounded displayed-snapshot inference contract; a run elapsed regression clears motion history, and it does not promise exact reconstruction of skipped intermediate moves. Camera following holds the last center when a followed collectible becomes inactive; manual editor navigation suspends follow without rewriting authoring data.

Standalone uses the same ordered per-GUID multi-atlas compositor as Arena, without changing Arena behavior. Linux export copies only the manifest/entry/sidecar/verified package closure; its trusted runtime must have explicit `collect-sprites` support. Export and headless/capture do not access player progress storage.

## Required verification

- Admission: missing/orphan GUID, region, clip, package, root/scene mismatch, UI, unsupported consumer; old primitive-only project remains valid
- Real editor assignment, undo/redo, sidecar save/reopen, pause/seek/Stop, unchanged authoritative state
- Runtime collection visibility, restart visual reset, missing follow target, owned-source loss, unchanged score digest
- Independent opaque atlas texel checks in runtime and composed EditorApp framebuffer, not only primitive viewport readback
- Copied built runtime/exporter, whole-source-hidden read-only export and byte-identical GPU output with unchanged project/export bytes
- Relevant default/mixed-feature strict Clippy, focused tests, independent exact-source review, then fresh mandatory feature CI and normal development integration

Physical native-window/Windows local tests, general UI authoring, source-linked prefabs/reimport, a new Collect project generator, and full P0 acceptance remain separate.
