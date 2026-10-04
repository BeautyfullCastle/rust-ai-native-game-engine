# Editor and 3D rendering: first follow-up scopes

Status: **design draft for #25, not implementation completion**. Prepared by
Orrery-Codex-Haneul on 2026-10-04 following
[the owner's request](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/31#issuecomment-5976416396).
The two child descriptions below are drafts inside this document; no child
GitHub issues have been created. Implementation requires the owner's scope and
file assignments after reading review.

The inspected source is `d9c689abd99a80f54e030c6e2a59b155eb86706e`, tree
`d39a08176ff125b2d1eef1427b055462c2ec5b6b`. The historical Windows measurements
are from `120efa6efa4b7ba6c82a88be7cb402e1e69ca2a1`; they are not fresh D9
measurements. Draft PR44's local Arena work and draft PR45's client queue bounds
are separate, unmerged changes at this draft's preparation time. They are not
part of this source baseline. This work reads source and writes this one document;
it runs no new builds, tests, host, GPU or performance campaign.

## Current capabilities and choice

The [editor design](design-v1.md#73-기본-레이아웃) describes a hierarchy,
inspector, viewport and history. The current editor has a flat entity list,
single `Option<Target>` selection, GUID-aware targets, field editing, transactional
dragging, proposal preview, undo/redo and a read-only replay Viewer. These already
exist; the first child extends selection and a discrete edit, rather than replacing
the editor or adding a scene relationship model.

The current 3D renderer already has instanced sphere/box/capsule/plane meshes,
one sun shadow pass, depth-tested debug lines, material lighting and LOW/default
presets. A 5x7 bitmap font and a 2D overlay over 3D already exist, as do cross and
arrow primitives. The first render child isolates sphere geometry detail. Cascaded
shadows, a contact-feed overlay and a new font system are later scope decisions.

| Inspected source | Existing contract to preserve |
| --- | --- |
| [editor model](../crates/orr_editor/src/model.rs), `Target` / `EntityRow::target` | GUID is preferred; a live entity handle is a fallback, not persistent document identity. |
| [editor controller](../crates/orr_editor/src/editor.rs), `selection`, `can_mutate`, `set_preview` | Existing single-target callers, Viewer protections and preview state remain supported. |
| [editor app](../crates/orr_editor/src/app.rs), hierarchy / inspector / viewport | Mutation controls already check permissions and proposal preview; model/API checks must also enforce them. |
| [document transactions](../crates/orr_edit/src/doc.rs), `apply_batch` | Existing all-or-nothing operations, one undo entry, rollback on invalid operation. |
| [ERP dispatch](../crates/orr_remote/src/dispatch.rs), `tx.*` / `world.patch` | These RPCs exist. A one-request batch edit RPC does **not** exist in this baseline. |
| [3D renderer](../crates/orr_render/src/renderer3d.rs), `Settings3D` / mesh upload / draw | Default requests 4x MSAA, 2048 shadow map, 32 segments; LOW requests 1x, 512, 12. Both passes currently share fixed meshes. |
| [3D render list](../crates/orr_render/src/list3d.rs), `Instance3D` | 80-byte instances, four mesh vectors and lines; no stable entity ID or LOD state. |
| [render counters](../crates/orr_render/src/stats.rs) | Submitted draw/pass/upload counts and CPU prepare/encode/submit intervals; not visible-triangle or GPU execution measurements. |
| [font](../crates/orr_render/src/text.rs) / [targets](../crates/orr_render/src/targets.rs) | Existing 5x7 text and 2D-over-3D composition remain available. |

## Child draft E1: GUID multi-selection and one atomic nudge

**Proposed title:** Editor: GUID multi-selection with one undoable position nudge.
User result: select several document entities, apply one discrete 2D position
offset, and undo or redo the whole edit once. The first edit is restricted to the
game adapter's existing editable position descriptor; it is not a generic transform
gizmo, continuous multi-drag, 3D hierarchy or component-wide bulk inspector.

### Identity, UI and document groups

Keep the existing single-target accessor as the primary selection for compatibility;
add a bounded GUID selection collection and explicit primary GUID for this child.
Ctrl-click toggles a row, normal click replaces the selection, Escape clears it.
The first limit proposed for owner review is 128 distinct GUIDs. Deduplicate and use
a stable order when constructing the batch; oversized selection is explicitly refused.
An entity without a document GUID may still be inspected through the existing path,
but cannot enter this document-edit batch. Re-resolve GUIDs after a preview rebake;
never reuse an old entity slot as the identity of a different entity. A scene load,
host replacement or document replacement clears the collection; a removed GUID
cannot silently become another target.

“Document groups” mean a view-only projection such as grouping rows by component
type or a transient user folder. They do not introduce parent components, ECS
relations, inherited transforms, serialization or checksum changes. Read-only
projection is a possible later E2 child; E1 retains the flat list with multi-selection.
Persisted folders, reparenting and cross-document references require separate design.

### Proposed API and transaction boundary

Reuse `EditorDoc::apply_batch` in a **new additive ERP endpoint**, tentatively
`world.patch_batch`, with a matching discovery/schema entry. The editor submits one
request containing GUID/position patches, label and expected document checksum.
The endpoint and its name are proposed, not implemented APIs. Retain the existing
single-field and transaction APIs.

The host validates Edit mode, capability, transaction availability, expected checksum,
distinct existing GUIDs, editable field descriptors and every checked fixed-point
value before publishing success. The proposed first request bound is 128 position
operations and 64 KiB of serialized request bytes, further restricted by any lower
host/client limit. Limits are checked before retained batch allocation/application;
failure returns a bounded explicit error and changes no document/history state.
Reuse the game's reflected position schema and checked FP conversion; display-space
floats must not enter simulation state. Zero displacement records no undo entry.
One nonempty successful batch records one entry with the original client origin;
undo and redo restore/reapply the complete set and preview frame.
The handler closes its internal transaction on every success/error return and
integrates with existing transaction ownership/effects; it cannot leave a transaction
open until timeout. Return one bounded summary/checksum, not an unbounded collection
of per-entity error strings. Discovery and host authorization enforce SceneEdit.

Separate `tx.begin`, N `world.patch` calls and `tx.commit` can group undo, but are
not a one-request atomic transport contract. E1 therefore needs the additive host
endpoint; it must not label a client-side sequence as an atomic RPC. RPC admission
failure is explicit, with no optimistic document mutation. If a request was accepted
and the connection becomes terminal before its reply, the commit outcome is uncertain:
stop editing, establish a fresh fenced connection and read the authoritative document
and history. Do not automatically resend or assume rollback. This follows PR45's
proposed queue contract once that change is integrated; it does not claim PR45 is D9.

Disable and reject the operation in replay Viewer, Play (including paused Play),
proposal preview, disconnected state, a foreign/open transaction or stale checksum.
Selection/navigation can remain read-only there. Enforce this in controller and host,
not just by greying a widget. Proposal acceptance remains the existing separate path.

### Acceptance for a future E1 implementation

1. Two GUIDs with different starting positions receive exactly the offset through
   one request; history grows by one. Undo restores the original scene/preview
   checksum and positions; redo restores the committed result. No-op changes none.
2. Invalid/missing GUID, uneditable or overflowed value, duplicate target, 129th
   operation, byte-bound overflow, stale checksum and open/foreign transaction all
   reject with zero partial edits, zero new history entries and unchanged checksum.
3. Rename and preview rebake preserve GUID selection across changed entity handles;
   removal, scene replacement and stale replies cannot edit a substituted entity.
   Single-selection callers and existing drag transaction tests retain their behavior.
4. Viewer, Play, preview and disconnected cases reject both UI and direct controller/
   RPC attempts; an accepted-request connection loss does not trigger a retry.
   A fresh fenced read determines the result before further edits.
5. A real native-window fixture shows two highlighted selected entities, the nudge
   result and one history entry; undo visibly restores both. Retain actual framebuffer
   PNG, adapter/backend/size and scene/checksum evidence. An egui fixture or render-list
   test alone does not establish native presentation. Add the fixture to the existing
   editor native smoke path only after its owner's file handoff; preserve existing
   Edit/paused-Play and host checksum protections.

Future tests belong beside existing `orr_edit/tests/edit.rs`, ERP capability/transaction
tests and editor/native fixtures. Their names/commands are an implementation plan,
not new passing results. No golden/assertion weakening is part of acceptance.

## Child draft R1: opt-in two-level sphere main-pass LOD

**Proposed title:** Renderer3D: opt-in screen-size sphere LOD with fixed shadow detail.
Keep `new`, `with_settings`, `Settings3D`, `Instance3D`, `RenderList3D` and existing
`FrameStats` layouts/behavior compatible. Propose an additive constructor accepting
a view-only `SphereLod3D` policy and separate `SphereLodStats3D`; these names are
design candidates. Existing constructors keep LOD disabled with their fixed meshes.
No new shader, RHI feature, lighting field or simulation input is required by this scope.

Initial candidate pairs are default 32/12 and LOW 12/6 near/far segments, with a
conservative projected bound-radius cutoff of 6 physical pixels. These are quality
candidates to validate, not measured tuning. Validate far segments within 3..effective
near detail and a finite positive cutoff; equal detail collapses to one sphere batch.
Keep capsules, boxes, planes, debug lines and shadow casters unchanged.

Generate the optional far sphere mesh once at construction. At draw preparation,
classify each sphere using its enclosing world-space bound projected by the current
camera/viewport. A conservative first algorithm projects the corners of the enclosing
cube, using the maximum radius from projected center. Require valid finite projection,
positive clip w and no near-plane intersection; invalid camera/scale/projection,
near-plane crossing or zero viewport falls back to near detail. Cover perspective and
orthographic cameras. Reclassify on camera motion/resize; the first version is stateless,
without per-entity hysteresis, culling or entity-ID requirements.

Stable-partition into retained-capacity near/far staging, preserving all instance data
and order within each bucket. Upload the combined sphere slice once. Main draws each
nonempty bucket with its mesh; shadow draws the whole sphere slice once with the
original near mesh. Other kind writes/ranges keep their current contract. Report near/
far counts and encoded index invocations in separate LOD counters. Additional static
mesh bytes/startup allocation and CPU classification cost must be visible; fewer
indices alone do not establish a faster frame or lower RSS.

### Acceptance for a future R1 implementation

1. Near+far equals input count with no loss, duplication, material reassignment or
   mutation of the source list. Test cutoff equality (far), both sides, perspective/
   orthographic, resize, motion, fallback, empty/all-near/all-far/equal-detail cases.
   Warm bucket changes reuse staging and GPU buffer capacities.
2. Current sphere index counts are 4608/576/144 for 32/12/6 segments. For 1000 spheres
   split 500/500, candidate main index invocations are 2,592,000 default or 360,000
   LOW; shadow remains 4,608,000 or 576,000. These include existing pole-degenerate
   triangle slots and do not count visible triangles. Mixed spheres use two main
   draws and one shadow draw; a sole nonempty bucket uses one main draw. Sphere-only
   frame uploads remain three calls/80,512 bytes (two 256-byte globals plus instances).
   Construction uploads are separate. Preserve empty/clear shadow and resize counters.
3. Disabled and all-near paths are pixel-identical to the corresponding fixed-detail
   control on the same adapter/preset/format. Existing lighting, shadow, depth, lines,
   sRGB/linear, MSAA fallback and resized-empty fixtures remain required.
4. Candidate far spheres retain an isolated silhouette boundary within one physical
   pixel of the same-preset reference, center color within four channel levels and
   no missing object. Test offset centers/rotation, threshold crossing, mixed opaque
   occlusion and instance-color association. A receiver region unaffected by direct
   sphere occlusion keeps identical shadow pixels. Validate both presets separately;
   an old whole-image coverage tolerance is not sufficient LOD evidence. If candidate
   limits fail, narrow the cutoff/increase detail and retain the failing result.
5. Planned correctness matrix: required Linux software Vulkan with `ORR_REQUIRE_GPU=1`,
   native Windows hardware Vulkan and verified WARP/DX12 software separately, plus
   explicit browser WebGPU and WebGL2 fixtures with their supported MSAA. Auto backend
   selection proves neither browser path. Record unavailable lanes honestly; generic
   browser CI is not proof this future LOD fixture ran. Preserve LOW/default behavior
   with LOD disabled on each actually tested backend.

## Baseline and future measurement protocol

Use the [existing baseline contract](editor-render-baseline.md) and
[published Windows evidence](baselines/2026-10-03-windows-release/README.md).
That historical source produced 12 observations on RTX5070/Vulkan hardware and
12 on Microsoft Basic Render Driver/DX12 software, with actual adapter identities.
It includes the sphere grid, synthetic editor/UI fixture and ERP editor round-trip
case; none establishes future multi-selection or LOD performance. Existing hardware
evidence must not be relabelled unverified because a stale roadmap says so.

The renderer's old wall intervals measure CPU render-call return/backpressure.
Prepare/encode/submit counters exclude GPU completion, presentation and readback.
LOW/default change mesh detail, MSAA and shadow resolution together, so their old
numbers cannot isolate LOD speedup. Synthetic egui timings do not measure native
window frame pacing; an ERP round trip is not local input-to-photon latency.
The old post-batch readback includes copy/map/backlog and is not GPU frame time.
Old binaries were hashed before capture, without archived post-capture rehash evidence.

Before a future measurement, freeze source/tree/hashes, matching control and candidate,
features/profile/argv/cache/prebuild state, scene/hash/camera sequence, resolution/format,
requested+actual MSAA/shadows and actual adapter/backend/driver/software status. Reserve
the current #28 quiet lane and run one workload at a time. Use at least three independent
process/device runs per case, retaining first post-construction frame, 10 warmups and
30 steady samples separately. Keep the existing grid and a newly named mixed-size
fixture separate. Compare LOD on/off within each preset, fixture and adapter, never
by pooling hardware/software or modes. Preserve scheduled failures and all raw samples.

Return-mode measurements retain the existing no-per-frame-wait contract, with draw/
upload/index/reallocation counts and separately timed post-batch readback. Report
per-process median/p95/max and the explicitly defined aggregation; do not add stage
medians. A separate native completion-observed campaign drains prior work, records
t0 before draw, t1 at return and t2 after a proven successful `wait_idle`, and reports
return/wait/total intervals separately. Capture device errors/loss: current wgpu wait
discards its poll result, so the future harness must prove successful completion.
This serialized mode changes queue depth and includes CPU/driver/wait costs; it is
not isolated GPU execution time or FPS. Browser completion needs an asynchronous
backend-specific receipt; current wasm CPU fields are unavailable. Timestamp-query
instrumentation for isolated GPU duration is a separate capability-checked child.

E1 also needs a separate equivalent-fixture interaction plan (selection size, target
count, local/remote transport, displayed frame, undo state and real completion receipt).
Keep the old 49-body editor and 1049-body UI fixtures distinct from any new batch case.
Neither child can claim an improvement ratio, new product budget or p95 before the
authorized paired evidence exists. No measurement is executed by this design task.

## Ownership, sequencing and non-goals

1. Reading review and owner decision come first. This document alone is claimed by
   Haneul; #25 remains OPEN and its design acceptance is distinct from implementation.
2. Proposed E1 paths include editor model/controller/app, ERP dispatch/discovery and
   their focused tests; reuse edit core rather than rewrite it. These paths need new
   owner reservation/handoff, including coordination with Ddang's #17 native fixture.
   Do not change PR44 or claim its work by elapsed time. #5 feedback takes priority.
3. Proposed R1 paths include renderer3d/mesh, focused gpu3d tests and dedicated evidence
   documentation. Reserve them separately before implementation; no RHI/workflow/golden
   change is preauthorized by this draft. Publish actual child issues only by the owner.
4. Groups/relations, 3D transform hierarchy, full gizmos/workspace persistence, cascades,
   contact-feed plumbing, new fonts, PBR/postprocessing, automatic quality switching,
   simulation changes and production GPU timers are excluded from these first children.
   Shared lockfiles, roadmap/progress and other owners' reservations remain untouched.
5. Preserve existing assertions/goldens/required CI. Future implementation needs focused
   correctness tests, actual required CI and reading review; new passing results and
   engine/epic completion must not be inferred from this draft or historical green CI.
