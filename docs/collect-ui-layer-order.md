# Collect HUD layer ordering (acceptance in progress)

## User workflow and boundary

Select an existing node in the optional Collect UI document panel. Bring subtree
to front places that node and all its descendants after the other nodes on its
screen. Send subtree to back places a root before other nodes on its screen, or
a child immediately after its parent. The entire subtree retains its internal
order. This uses Orrery's existing flat draw order and parent-before-child
validation; it is not depth-first sibling stacking. No IDs, parent relationships,
geometry, content, screen, action, binding, schema or font bytes change.

During reorder and Undo/Redo, selection follows stable node IDs. The general
`apply_document` API retains its existing index-based selection semantics. A successful change is one bounded Undo entry;
Undo and Redo restore both order and selection. A no-op or rejected edit keeps
redo; a new actual edit discards redo. Save and reopen use the existing validated
font, admitted path, directory pin, external-edit detection and atomic-save flow.

No new optional package is needed: the code belongs to existing default-off
`collect-ui`. No new third-party dependency, Cargo registration, runtime/render
API, simulation/replay format or project/package authority is introduced. The
scope is Collect's authored HUD, not Arena/Room UI, generic reparenting or drag
and drop. During Play the controls remain disabled.

## Executable completion criteria

- Pure/order tests: interleaved descendants, nested root/child movement, all node
  fields unchanged, unrelated nodes retain relative order, all parents precede
  children; other screens and repeated edge moves are no-op where appropriate
- Invalid document/index and absent selection leave all document/history/selection
  state unchanged; no-op/rejection retain redo; actual edit invalidates redo
- Exact bounded 32-entry undo/redo and stable selection restoration; existing
  duplicate/add/remove controls remain coherent with the extended history
- Real accessible pointer widgets select/reorder/Undo/Redo/Save; reopen persisted
  bytes into actual CollectUi, and overlap routes a real press/release to the new
  topmost action. The new isolated panel read-only fixture covers the disabled controls, but
  actual EditorApp Play lifecycle, container clipping and keyboard focus still
  require the additional acceptance cases specified below
- Actual EditorApp author/save/reopen/Play pipeline with real GPU overlap pixels;
  gameplay Frame bytes/checksums invariant for identical authoritative input
- Export actual saved project, hide original source and make relocated bundle
  read-only; prove identical runtime pixels and hit/action routing, unchanged
  bundle inventory. Existing native-window/physical-GPU limits stay explicit
- Omitted reorder command must fail the expected order/pixel/action oracle; restore
  original source and rerun successfully. Never count a skipped GPU/export as pass
- Existing six panel tests, existing subtree-duplicate tests, fresh dedicated tests,
  default dependency isolation and strict all-target Clippy, independent review,
  mandatory exact-head CI and separately reviewed development union

## Initial source and ownership record (historical)

Base: development 84b5317abf67c91cb44a7a78a4f03a398821132d,
tree c388d487c813677940aca1c3ece145495f77176c. The candidate starts from its verified
`collect_ui_panel.rs` blob 94e76fef623b41331ce1862f818de9f568cdfdf8.
The three-file isolated stage changes only that panel, adds its direct private
`collect_ui_layer_order_tests.rs` child module, and this document. No original
checkout was edited during that initial stage. At that time, new integration
test/tool/workflow paths were future ownership requests. The later test and
hosted gate source is now implemented; its actual hosted execution remains pending. PR169/181 changed-file lists are disjoint. PR175's
latest record says its previous ownership completed development integration via
PR179; open historical source PR175 is not evidence of a new active assignment.
No competing layer-order reservation was found in latest #94/#106 comments/open
PRs; #2 records no new Haneul/Ddang assignment.

## References

- Existing flat draw contract: crates/orr_sample/src/authored_ui.rs
- Existing topmost pointer and keyboard/occlusion contracts: crates/orr_sample/src/collect_ui.rs
- Previous integrated subtree feature: https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/106#issuecomment-6107427554
- Workflow comparison only: https://docs.unity.com/en-us/engine/6000.6/manual/uitoolkits/uielements/uie-structure-ui/uie-visual-tree/uie-draw-order

Historical initial status: the original three-file stage was source-only. Current executed scope is recorded below. Broader #94/#106 stay open; #2's original 22-item denominator, main and PR1 stay unchanged.


## Original additional application acceptance plan (historical)

1. New integration case `editor_play_layer_order_readonly`: open an admitted
   generated Collect UI project in actual EditorApp; use real panel controls to
   reorder and save; enter Play through the actual mode action. Attempt Front,
   Back, Undo, Redo and Save at their observed rectangles. Assert unchanged UI
   bytes, full document and selected stable ID/history. Stop and repeat one edit
   to establish that the actual lifecycle restored editability. The isolated
   panel test's `editable=false` fixture is not substituted for this case.
2. New focused case `layer_order_clipping_and_container_occlusion`: overlapping
   lower button, clipped parent and oversized descendant. Compare pointer clicks
   in the descendant's visible intersection, its clipped-away region and the
   parent-only region before Front, after Back, Undo, Redo and save/reopen.
   Expected action identities and no-action occlusion are specified explicitly;
   passive labels remain pass-through. Real GPU captures must independently
   confirm the same visible overlap regions and clipping, rather than accepting
   action assertions as pixel proof.
3. New focused case `layer_order_keyboard_focus_and_pointer_ownership`: give a
   real button focus through Tab; Enter/Space only activates when its center is
   unoccluded. Reopen each saved reordered document and repeat. Press over one
   overlapping button and release over another must dispatch nothing. Focus loss,
   PointerGone and screen/mode changes must cancel the pending press. These use
   existing production CollectUi input APIs without mutating runtime internals.
4. New dedicated author/export test, script and workflow will be allocated only
   after parent review. Require exact named inventories for all new tests and the
   existing six `collect_ui_panel::tests` plus existing subtree duplication;
   require explicit GPU execution, actual production export with source hidden,
   read-only bundle and unchanged Frame checksums. Preserve the current required
   isolation policy. No-reorder test-only negative must fail the exact order,
   action or pixel oracle; restoration must restore source hashes and pass.
5. Intended focused CPU command after allocation is `cargo test -p orr_editor
   --release --features collect-ui --lib collect_ui_panel::layer_order_tests --
   --test-threads=1`. The original exact candidate has now passed the focused CPU scope below. Complete exact-head strict/default, integration/GPU/export and CI commands remain separately bound; a test-only source supplement does not inherit a fresh producer claim.

## Earlier source-review changes (historical)

Initial three-file stage and ownership draft/manifest are retained separately in
`hud-layer-order-stage-review0`. The successor adds deep interleaved/two-foreign-
screen tests, complete-history no-op/rejection snapshots, actual Add/Remove/
Duplicate→Undo→Redo selection tests, actual rejected Add-at-depth-limit and
font-failure-injected Remove atomicity, and disabled/no-selection widget tests.
The Remove failure deliberately corrupts test-owned font data: this is defensive
injection, not a claimed normally admitted project condition. Production source is unchanged from the initial staged candidate. The original exact candidate later executed all ten tests successfully; see the scoped results below.

Ownership observations were read 2026-10-11 23:44–23:46 UTC: #94 latest comment
6115205952 concerns PR169 diagnostics; #106 comment6107427554 and #175
comment6109823467 record HUD/Arena development integration via #179; #2 records
external-owner handoff closure. Current open PR titles and PR169/181 changed paths
were checked in the same window. These read-only observations are bounded research,
not a new ownership reservation; refresh latest ownership before public posting.


## Verified focused CPU scope and remaining work

On candidate `00159a57e63a0eb7e2112016c4db4e47044a7b61`, tree
`41f49b6a9a2138a2dc1b4a985277b2294468311c`, 23 unique CPU tests passed:
ten new layer/history/widget cases, six existing panel cases, and seven existing
subtree-duplicate CPU cases. An omitted reorder command failed the exact expected
singleton assertion; source restoration produced a full byte-identical positive
ELF and repeated the ten new tests successfully. The ten repeats are not ten
additional unique cases. Independent composite review is SHA256
`1d336f591e0f09796730c627045eaafe4052d6bf43e937f84487e0c124ae615f`.
All original attribution/allocation STOPPED attempts remain separately retained.

The additional test-only application fixture uses existing Linux
`collect-ui,project-create`: actual EditorApp authoring/Play-disabled controls,
history/Stop/reopen, production CollectUi clipping/keyboard/pointer ownership, and
91 deterministic Frame byte/checksum samples. It is not yet compiled or executed.
On the distinct Phase B candidate `7127c43e4487f3dd67b676ec5ee25c910019fb0e`,
tree `7da5a16467dbade41387597ef4479e6f8bc0225d`, default/no-default complete
library inventories, absent optional Collect-panel inventories, two full archive
readbacks, default/no-default/collect-ui all-target strict Clippy, and parsed
895-row default dependency isolation passed. Independent composite review is
SHA256 `e2cc95e83f5c45b156aad202e2ee97620348b190c5d32756e9b13b9a7c81bde3`.
Earlier stopped attempts and their separately reviewed continuations are preserved.

The maintained hosted gate and a later bounded GPU test-fixture delta are prepared
and source-reviewed. These later bytes do not inherit an executed Phase A or B
claim. The dedicated `collect-ui,project-create` producer, its three actual
EditorApp/runtime cases and combined strict check, plus positive/order-negative/
restored software-GPU and source-hidden read-only export acceptance, must pass
on the exact published head before merge. See [the mandatory hosted gate](hud-layer-order-ci.md).
Local combined acceptance remains blocked; no skip is counted as a pass. No full
feature completion or development integration is claimed.

EditorApp currently exposes the Collect authoring panel, not a live Collect HUD
viewport. Pixel/overlap/clipping acceptance therefore uses the production Collect
runtime compositor; actual EditorApp acceptance covers editing and lifecycle
protections. A live HUD editor preview is a separate discovered authoring gap, not
part of this change. Native-window, physical-GPU and real-device acceptance are
not implied by software-GPU/offscreen evidence.
