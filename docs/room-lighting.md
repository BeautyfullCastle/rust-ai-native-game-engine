# Saved Room point lighting

This implementation was reconstructed as `347be9a0b4d13070e7bdaa5cd7c615cdbf4a5a14` after the previous executor was lost, then overlaid on repaired camera source `28a2a48e3d116d35645e3baee434a7fc3f145ec4` (tree `cc7873e174432a273019a6bc710fb3e9d557d1a8`). The overlay preserves the camera package sequence, fixture, atomic source-retirement and WSS repairs. It is not the historical `245a619` source tree. Earlier recovered test receipts remain attributable only to their recorded source; the repaired candidate was independently reviewed and checked as recorded below.

`room-lighting` is default-off in `orr_sample` and `orr_editor`. A Room project may declare a `lighting` path in its entry, alongside its scene and models. Consumers explicitly opt in; older admission wrappers reject a declared light even if another dependency enables the feature. An absent document retains legacy light-off rendering.

The sidecar is a strict, versioned object, at most 4096 bytes:

```json
{"version":1,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12}}
```

`point_light: null` explicitly disables it. All fields are required. Unknown fields, duplicate keys, positional object arrays, unsupported versions, invalid numbers and oversized input fail admission. Numeric limits match `orr_render::PointLightSettings`: finite positions within ±1e9, nonnegative linear RGB and intensity up to 1e4, range from 1e-6 through 1e9. This is one fixed, unshadowed, stylized diffuse light in world coordinates. It does not add spots, entity attachments, animation, GI, a baker, PBR or photometric units.

The portable root-level sidecar path must differ from the scene, manifest, package lock and other presentation documents. Existing symlink and containment restrictions remain in force. Manifest and sidecar bytes are pinned during admission; export copies the exact admitted bytes and includes them in content identity. Immutable installed package contents are not edited.

The actual Room editor panel stages values and applies them separately. Apply, Undo and Redo update only lighting presentation, using an independent history of at most 32 entries. Save retains that history. The panel binds immediately to the admitted scene path and the editor source lifetime; source replacement or restart retires the old panel even before its first UI frame. Play, preview, remote sources and incoherent snapshots cannot author or save lighting.

Save checks the original sidecar and manifest bytes, regular-file policy and pinned directory identity before atomic replacement. Pre-persist failures preserve the file, saved baseline, history and dirty state. If replacement succeeds but directory sync fails, the panel reports uncertain durability and advances its baseline so retry does not misidentify its own write as an external edit. These checks detect ordinary staleness; they do not claim hostile filesystem-race isolation. The directory handle follows the existing camera-panel implementation. Linux execution must be proved freshly; Windows/macOS directory-handle authoring has not been established.

Both native Room runtime paths and the actual editor viewport pass the same validated settings to static and optional skinned imported-scene preflight and draw. The renderer/shaders and simulation production code remain unchanged. Lighting must not alter serialized Frame bytes, gameplay state or checksums through drawing, Play, stepping or seek.

## Fresh verification gates

Run each under the coordinator's shared Cargo/disk lease. Results below are tied to the exact tested source and do not imply a hosted CI pass.

- Sample document library test: `room_lighting::tests::strict_bounded_renderer_compatible_document`
- Sample `room_lighting` integration target with `room-lighting,project-create`; repeat with `room-project,project-create` for disabled-capability rejection
- Editor `room_lighting_panel::tests` and actual `room_lighting` integration with `room-lighting,room-character,project-create`
- Required ignored actual viewport test: `generated_room_lighting_actual_viewport_gpu_static_and_character`, with `ORR_REQUIRE_GPU=1`
- Real production bins: `cargo build --locked --release -p orr_sample --features room-lighting,room-character,project-create,project-export --bin room_escape --bin orr_export_room`
- Required ignored export singleton: `room_lighting_source_hidden_readonly_export` in sample `room_lighting_export`, same features, with `ORR_REQUIRE_GPU=1`, `ORR_REQUIRE_PROJECT_ISOLATION=1`, and absolute `ORR_ROOM_RUNTIME` / `ORR_ROOM_EXPORTER` paths to freshly verified production binaries

The export fixture must live outside the owning workspace. Real `bwrap` hides the workspace, original tools and authored source, relocates each bundle and makes it read-only. Static and character off/on/moved captures must match their source equivalents exactly, retain matching initial/movement/key checksums, show meaningful lighting and model differences, preserve installed packages and reject malformed inputs and write attempts. Set `ORR_ROOM_LIGHTING_CAPTURE_DIR` to retain captures and evidence.

The workflow also selects the separate checkpoint-inclusive profile, existing renderer lighting cases, compatibility and strict Clippy. A pass on the minimal local profile does not prove the hosted profile or remote CI. Native-window, physical-GPU and cross-platform results require their own explicit evidence.

## Repaired-source verification (2026-10-10)

The executable source was checked at `6252c2098ea7e7a13634c32bcf139d9b36ae40f5`, tree `6e22b7dfb7e300d7758f5a1bc9bbd3bbc36d2ee2`. The final publication cleanup changes documentation and one leading module comment. It also inherits the separately verified camera CI inventory repair and adjusts its package counts for the new lighting regression; executable Rust code and Cargo manifests remain unchanged. All 21 local gate invocations completed normally with Cargo/guard exit 0 and build-finished success; all 1,224 tracked-file hashes were unchanged around each run.

- Package library: 37 passing cases, including preserved legacy sequence positions and lighting object/null/duplicate rejection; its expected child-process singleton also passed
- Default boundaries: editor/sample check, dependency tests 5, lighting-disabled admission 3
- Lighting: document 1, admission 5, panel 9, minimal actual editor lifecycle 3
- Checkpoint/character compatibility: sample material 2, character 3, camera 4, lighting 5, Room project 10; mixed editor material 8, character 3, lighting 4, Room project 4
- Required point-light renderer GPU cases: 5; required actual static/character editor viewport case: 1
- Checkpoint-inclusive production runtime/exporter build and genuine source-hidden/read-only export case: 1
- All seven workflow strict all-target Clippy configurations passed without warnings

All 14 editor and 42 export PNG hashes were checked; all 18 source/export capture pairs are byte-identical. Actual adapters report `llvmpipe (LLVM 19.1.7, 256 bits)` through local GL software. Export uses real `bwrap` isolation and verifies read-only relocated bundles, source hiding, simulation identity and malformed/write rejection. Frozen production bytes have verified fresh-producer and exact-feature lineage.

Five inherited compatibility cases remain ignored: one sample Room project case and four editor material/character/Room project cases. They are not included in the passing counts. The dedicated required lighting GPU/export cases ran explicitly with zero ignored cases. Hosted required CI, native-window, physical-GPU and Windows/macOS execution are separate outstanding evidence; no full-workspace or development-integration pass is claimed here. The separately reviewed camera inventory repair is inherited from `dad3c087` (published camera `681b44f4f413186a947c7dff18123474102d99ad`, tree `5bbf26263b1c383efb7d6278ff522a98bb498083`). Its exact-source runtime evidence covers the inherited creator/UI/sample inventories. Lighting additionally names its new always-on package regression in every exact package inventory and requires the already verified package parent count of 37 plus the FIFO child count of 1. These CI/checker metadata changes do not alter executable Rust or Cargo manifests; hosted checks on the final published head remain mandatory.
