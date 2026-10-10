# Authored sprite orientation

The optional Arena/Collect sprite path now consumes the existing renderer's
per-instance rotation and mirroring. It adds no simulation components, collider
changes, tint, shader, input or dependency changes. Default builds remain
independent of this path; existing sprite-enabled builds admit the new descriptor.

Select one bound actor in the matching local Edit scene. Expand **Sprite bindings
(view only)** and **Selected sprite orientation**, then choose **Sprite quarter
turns**, **Mirror sprite X**, or **Mirror sprite Y**. Each interaction immediately
commits one whole-document binding-history transaction. There is no retained
per-target draft. Controls require a coherent current GUID/full-entity join;
Play, previews, remote/viewer sessions, missing entities and an active scene
transaction reject edits. The atlas preview and actual editor quad use the same
saved orientation as standalone gameplay and export.

Mirroring uses the original atlas axes first. Rotation then turns the resulting
quad counter-clockwise around its center in world coordinates. The region's
unrotated width/height and world-units-per-pixel stay unchanged; odd quarter turns
therefore exchange its visible extents. Collision/picking geometry is unchanged.
The 16 authored combinations represent eight distinct geometric transforms:
adding two quarter turns while toggling both mirror flags gives the same result.
Both representations remain valid and retain the author's exact choices.

The first orientation edit upgrades the sidecar to version 3. Its optional
per-binding `orientation` object contains exactly three required fields:
`quarter_turns` (integer 0–3), `flip_x` (boolean), and `flip_y` (boolean). The object
must be a JSON map, not a sequence. Missing members, nulls, duplicate/unknown
members, wrong types and out-of-range values fail admission. Versions 1 and 2
reject any orientation field, including an explicit identity object or null.
Absent orientation remains identity: zero rotation, no mirrors and unchanged
white tint. Reading legacy documents does not migrate them.

**Reset sprite orientation** removes that binding's orientation field and keeps
a version-3 document at version 3. **Undo binding** restores the complete previous
document, including its old version; Undo of the first edit can return exactly
to version 1 or 2. Repeated identical changes do not create history. Asset/clip/
scale reassignment retains each target's orientation; unselected bindings keep
all their values. **Use selected sprite settings** explicitly copies orientation
into the assignment draft, including identity, and **Assign sprite to selection**
then applies those settings together in one undoable transaction. The draft
preview and text show that choice. **Keep target sprite orientations** cancels
only that copied orientation; normal reassignment again preserves each target.
Opening/creating/closing bindings or directly editing orientation clears that
copied override. Explicit removal deletes the complete binding as before.

Every orientation change revalidates the installed package and compares its
canonical sprite document and decoded image against the displayed cached asset.
Changed or invalid sources reject the transaction and retain authoring/history.
A valid externally installed replacement requires **Reload assets** before an
orientation edit. Explicit PNG reimport retains saved v3 sidecar bytes and updates
the preview through its existing transaction. Save/reopen and export retain the
new fields; scene Save and sidecar Save remain separate operations.

The new acceptance targets separate schema/history checks from actual pixels:

- `orr_sample --test sprite_orientation` checks closed admission, all 16 serialized
  choices, the eight geometric equivalences, authoritative Frame/checksum
  preservation, pause/seek/restart, collection visibility and source ownership
- `orr_editor --test sprite_orientation` checks real widgets, per-actor assignment,
  Reset/Undo/Redo, legacy byte restoration, Save/reopen, interrupted source loading,
  source reimport and Play/Stop fences
- Explicit ignored GPU cases require actual runtime and composed-editor readbacks
  of an installed asymmetric 5×3 region inset within a 7×5 atlas. The independent
  oracle uses integer quarter-turns and source texel colors for all combinations;
  editor proof covers both Arena and Collect
- The explicit ignored export case requires trusted production Arena/Collect
  runtimes and exporters, bubblewrap source hiding, software GPU and a relocated
  read-only bundle. It checks exact descriptor bytes, identical production PNGs,
  and unchanged project, package-source and bundle bytes, with no fallback

Focused CPU commands (only when the shared build lease is assigned):

```sh
cargo test --release --locked --offline -p orr_sample \
  --features project-export,collect-sprites,image-reimport --test sprite_orientation
cargo test --release --locked --offline -p orr_editor \
  --features sprites,collect-dodge,image-reimport --test sprite_orientation
```

For pixel acceptance, run each target's `all_orientations_render` test with
`--ignored --exact` using its full test name; both cases fail if their GPU route
is unavailable. `ORR_ORIENTATION_CAPTURE_DIR` retains PNG evidence. Export needs
`ORR_ORIENTATION_ARENA_RUNTIME`, `ORR_ORIENTATION_ARENA_EXPORTER`,
`ORR_ORIENTATION_COLLECT_RUNTIME` and `ORR_ORIENTATION_COLLECT_EXPORTER` pointing
to trusted production binaries built from the reviewed source. Its full name is
`authored_orientation_export_relocates_with_sources_hidden_and_read_only`.
`ORR_EXPORT_HIDE_ROOT` can specify the verified workspace ancestor to hide.

Implementation, executed CPU/GPU/export proof, strict default/mixed-feature lint,
independent review, required exact-head hosted CI and development integration are
separate gates. Native window, physical GPU and Windows-local proof are outside
this bounded slice unless separately recorded. No whole-roadmap closure follows
from these tests.
