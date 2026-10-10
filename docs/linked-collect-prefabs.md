# Linked Collect groups (first bounded authoring profile)

This is a partial #105/#94 feature, not generic prefabs or asset reimport.
The optional `linked-prefabs` feature adds source-linked CollectDodge authoring.
Default builds keep the previous scene format and dependencies. The simulation
continues to consume a fully baked initial frame, never a source file.

## Boundaries

- Profile `collect-actors-v1`: 1–8 non-player collectible/hazard actors per source,
  up to eight linked instances, within the existing game actor caps.
- Source GUID set, component structure, kind and source ordinal cannot change in
  an update. Unsupported structural changes reject, rather than dropping data.
- Each instance allocates fresh GUIDs and contiguous per-kind ordinals. Ordinal
  assignments are separate immutable instance bindings, not user overrides.
- Only `CollectDodgeV1::Actor.position` has a user override. Once marked,
  subsequent ordinary position edits retain that marker; other unexplained
  edits are rejected. Revert restores the current baseline position.
- Source position, velocity and names may update when admitted by the game.
  Explicit position overrides survive these updates. Each instance is updated
  explicitly, not by a filesystem watcher or automatic global propagation.
  Replacing the source file is a separate explicit file operation; document
  undo restores an instance and its metadata, not that external source file.
- The source identity is a bounded normalized relative path, an inert label.
  Core APIs accept owned source bytes. They never open or trust that label as
  filesystem authority. The first editor panel accepts flat filenames beside
  the current scene. Capture creates a new file only; replacing an existing
  `.prefab.yaml` requires a separate explicit action and unchanged admitted bytes.
  Checked/pinned local I/O is not a hostile-filesystem-race sandbox.
- Linked actor groups do not inherit sprite/UI sidecar assignments. New actors
  retain the existing procedural fallback until separately assigned in sprite
  authoring; this slice does not claim art/presentation prefab inheritance.
- Nested linked sources, player copying, additions/removals of source entities
  or components, generic field overrides, 3D imported props and reimport are
  outside this slice. Instance detach/removal is not a separate command yet;
  undoing its creation restores the previous document atomically.

## Persistence and validation

Unlinked scenes serialize as byte-compatible `orr.scene/1`. Linked scenes use
explicit `orr.scene/2` and require the consuming build's linked-prefab capability.
The scene contains both flattened entities and typed metadata: canonical source
baseline and SHA-256, source-to-instance GUID mapping, ordinal assignments and
explicit position override markers. Unsupported readers reject version 2.

There is no scene/metadata sidecar transaction. A single document operation stages
entities and metadata together, retains individual op type/reference checks,
then performs one final bake and host admission before publication and one undo
entry. Metadata-only source updates still change revision and history. Undo/redo
restore baseline provenance together with entities. Failed admission preserves
the live scene, preview frame, allocation, history and redo.

Parsing and baking independently validate baseline canonicality and hash,
injective/disjoint/live GUID mapping, exact Actor reflection profile and the
flattened instance proof. Baselines parse only as entity-only version 1: recursive
metadata is prohibited. Existing CollectDodge raw/canonical scene limits remain
64 KiB, with at most 32 KiB aggregate embedded baselines. Metadata consumes this
budget; the bounds are not relaxed.

The local source file may disappear after editing. Save/reopen, initial bake,
play/restart, and exported runtime use the self-contained admitted scene. An
explicit future update needs a readable source again. This is not an arbitrary
code loader and does not create new game/build/network identities.

## Verification contract

Required focused coverage includes two independent instances, ordinal allocation,
actual editor widgets, explicit override/revert, source changes and rejected
structural conflicts, metadata-only undo/redo, failed-operation full-state
preservation, same-file save/reopen, deleted-source runtime, feature-disabled
rejection and unchanged scene-1 output. Focused results, independent review,
source publication, exact-head required CI and development integration are
separate checkpoints; this document alone claims none of them passed.

Design references: [Godot scene instances](https://docs.godotengine.org/en/stable/getting_started/step_by_step/instancing.html)
and [Unity 6 instance overrides](https://docs.unity3d.com/6000.0/Documentation/Manual/PrefabInstanceOverrides.html).
Their broader functionality is not claimed here.
