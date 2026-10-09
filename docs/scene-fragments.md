# Reusable scene fragments (partial #105)

`orr_edit::SceneFragment` captures a closed entity selection as a reusable,
entity-only `orr.scene/1` file. Instances are independent copies. This is a
headless authoring foundation, not completion of the prefab/editor milestone.

## Runnable authoring workflow

From the repository root:

```sh
cargo run -p orr_edit --example scene_fragment -- /tmp/orr-fragments
```

The example loads the existing `scenes/physics_demo.scene.yaml`, captures
`body_01` and `body_02`, saves and reloads `body-pair.scene.yaml`, and inserts two
copies with different offsets. It edits one copy independently, undoes/redoes
the edit and a whole instance, saves `instanced.scene.yaml`, reopens it, and
checks identical checksums across two 60-tick headless play runs. It writes
only into the explicitly supplied output directory; no repository scene is
modified. This is CPU/headless evidence, not native GUI or GPU validation.

## API and semantics

- `SceneFragment::capture(&Scene, &[Guid])`: exactly the requested selection;
  rejects empty, duplicate and unknown IDs and outbound references. Inbound
  references from unselected entities do not enlarge the selection. Source
  singletons and comments are omitted
- `SceneFragment::to_yaml()` / `from_yaml(text, &TypeRegistry)`: deterministic
  existing strict scene format, with no new asset format. Loading rejects
  singleton payloads, unknown types/fields, duplicate keys, invalid ranges and
  unresolved references
- `EditorDoc::instantiate_fragment(&fragment, placement, origin)`: returns a
  `FragmentInstance.guids` mapping from original fragment IDs to fresh IDs
- `FragmentTranslation2D { component, path, delta }`: caller-selected reflected
  Vec2 field, such as `orr_physics::Body` / `pos`. Components that do not match
  are untouched; at least one must match. Missing fields, non-Vec2 fields,
  checked fixed-point overflow and reflected range violations reject the copy

Allocation is deterministic in source GUID order, starting at the document's
allocation cursor and excluding all live and original fragment IDs. It never
wraps the u32 ID search; exhaustion is an error. A repeated insertion into the
same live document receives disjoint IDs. Undo/redo preserves recorded IDs.
The ordinary allocator skips IDs occupied by successful copies.

Only typed `Value::EntityGuid` references are recursively remapped through
arrays, structs and tagged values. Null references remain null. Raw runtime
`Value::Entity` handles are rejected, even `Entity::NONE`; runtime handles are
not stable authoring identities. Other scalars (including asset IDs) remain
unchanged. Self, forward, repeated and mutual references are valid graphs.
No hierarchy or ownership semantics are inferred from field names.

All empty entities are spawned before any components are added, using one
`apply_atomic_batch`. Undo removes every copied component before despawning
any copied entity. A successful copy is one history entry; any error leaves
live scene, preview frame/index, history/redo, dirty state, revision and GUID
allocation unchanged. Instantiation cannot join an open transaction.

## Bounds

These are new fragment-specific bounds, not general scene API limits:

| Resource | Limit |
| --- | ---: |
| Encoded YAML, checked before parse | 1,048,576 bytes |
| Selected entities | 1–256 |
| Total components | 2,048 |
| Recursive value nodes | 65,536 |
| Value nesting depth (root = 0) | 32 |
| Individual names/strings/field paths | 4,096 bytes |
| Aggregate names/value strings before copying | 1,048,576 bytes |

The existing strict YAML parser additionally limits YAML nesting to 48 and
rejects aliases, anchors, tags, multiple documents and duplicate keys.
Capture validates value structure before copying it; instantiation validates
component schemas in an isolated staged document before live replacement.

## Explicitly deferred

No linked-prefab source updates, tracked source-relative overrides, reimport
conflict resolution, package ownership/versioning, editor panel, asset
registry changes, hierarchy/root transforms, rotation, scaling or 3D
placement. Ordinary `Op::SetField` edits provide independent per-copy changes
and existing undo/save behavior; editing the fragment does not update copies.
Asset record cloning/moving is separate from entity graph instantiation.
