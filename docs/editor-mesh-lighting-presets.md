# Portable Mesh lighting presets

The **Mesh game lighting** inspector can explicitly save and load a small,
data-only lighting preset across local MeshNavigation scenes. This is an
optional editor workflow under `mesh-navigation`; it adds no renderer, runtime,
export, or deterministic simulation behavior. Scene-local lighting remains
owned by the existing Apply/Undo/Redo and workspace-save workflow described in
[Mesh scene lighting](editor-mesh-lighting.md).

## Save an applied look

1. Open a saved local MeshNavigation scene with an admitted graph
2. Author lighting and click **Apply game lighting**
3. Enter an absolute local filename ending in `.mesh-lighting-preset.json` in
   **Lighting preset file**; its parent directory must already exist
4. Click **Save applied lighting preset**

Save exports the current valid **applied** lighting, including an applied state
whose scene-local lighting history has not yet been saved. It does not export
unapplied text, silently apply default lighting, change history, or save the
scene or its lighting sidecar. The file must be new: an existing destination,
including a link or directory, is refused without replacement. Use a different
filename to save a revision.

## Stage the look in another scene

1. Save the source workspace if needed, then open the target MeshNavigation scene
2. Apply the target's lighting first if it has no applied or saved lighting
3. Resolve any current lighting form draft before loading a preset
4. Enter the preset's absolute path and click **Load lighting preset**
5. Review the staged form, then click **Apply game lighting** to preview it
6. Use **Undo game lighting** or **Redo game lighting** as needed
7. Click **File → Save mesh workspace** to persist the selected applied state

Load replaces only reusable form fields: direction, light/sky/ground colors,
intensity, ambient, shadow enablement, tone-mapping enablement, and exposure.
The target scene's `shadow_center` and `shadow_radius` remain its own, including
the exact existing strings in those form controls. A preset cannot carry
coverage bounds from a differently sized scene.

Loading alone does not change the applied lighting, rendered viewport, lighting
history, simulation Frame, scene files, or lighting sidecars. It creates an
unapplied form draft. The existing Play and workspace-save guards remain in
force until that draft is explicitly applied or discarded. A legitimate
unsaved **applied** history does not block preset staging; loading preserves
that history until the next explicit Apply.

Dirty form drafts, unavailable or stale scene/epoch/graph contexts, invalid
lighting, and external sidecar conflicts are refused. A failed or corrupt load
leaves all draft fields and history intact and reports the error. Preset path
entry itself is not a lighting edit.

## File contract and local-file limits

Presets are strict schema 1 JSON with identity
`ORR_MESH_LIGHTING_PRESET_V1`. The only payload is reusable lighting values;
there is no scene filename, graph revision, shadow coverage, asset reference,
path, script, URL, or network access in the payload. Unknown, missing, duplicate,
or incorrectly shaped fields are refused. Values use the existing finite,
bounded scene-lighting validator. The entire file is limited to 16 KiB.

The editor accepts only bounded local files through real, existing directory
ancestry and refuses symlink/reparse-point paths. Save validates before
publishing a new file without clobbering an existing destination. Use a trusted,
stable directory: these checks do not claim strict filesystem isolation against
hostile concurrent mutation, or power-loss durability. Loading a preset does not
create an exportable preset dependency; after explicit Apply and workspace Save,
the normal scene-local lighting sidecar is authoritative.

## Acceptance checks

The CPU tests cover staging into unsaved applied history, exact shadow-string
preservation, no hidden Apply/save/history changes, new-only save, invalid input,
dirty forms, stale identity guards, external sidecar conflicts, and the existing
Apply/Undo/Redo/workspace-save path.

```sh
cargo test -p orr_editor --features mesh-mission --lib mesh_lighting_preset
cargo test -p orr_editor --features mesh-mission --lib mesh_lighting_panel::presets::tests
```

The opt-in integration test drives production `EditorApp` widgets using synthetic
egui input and a **software Vulkan** adapter. Configure a software Vulkan driver
first and choose a fresh absolute capture directory that does not exist:

```sh
WGPU_BACKEND=vulkan \
ORR_MESH_LIGHTING_PRESET_ACCEPTANCE=1 \
ORR_MESH_LIGHTING_PRESET_CAPTURE_DIR=/absolute/new/preset-evidence \
cargo test -p orr_editor --features mesh-mission --test mesh_lighting_preset_ui -- \
  --ignored --exact portable_lighting_preset_cross_scene_stage_apply_undo_save_and_fresh_reopen \
  --nocapture --test-threads=1
```

It authors and applies scene A's look, saves a preset without saving A's workspace,
refuses a second save to the same preset, opens scene B with a distinct graph,
and proves that loading changes only the draft. It compares exact viewport pixels
before/after staging, then verifies visible Apply, exact Undo/Redo, preservation
of B's shadow coverage, workspace Save, corrupt-load preservation, and a fresh
Physics-editor open of B with the same persisted pixels. Eight viewport PNG captures, one full-editor screenshot with preset-control
and status bounds checked inside the 340-pixel inspector, and
`receipt.json` record these checks. They are software-rendered editor evidence,
not hardware-GPU, physical operating-system-input, standalone-player, or export
acceptance.
