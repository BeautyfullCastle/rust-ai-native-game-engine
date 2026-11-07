# Portable Mesh lighting presets

The **Mesh game lighting** inspector saves and loads data-only lighting looks across local MeshNavigation scenes. Build with `mesh-navigation`; authoring tests also use `mesh-mission`. Scene lighting remains owned by the existing Apply/Undo/Redo/workspace-save workflow. Renderer, shader, runtime and deterministic simulation behavior are unchanged.

## Save an applied look

1. Open a saved local MeshNavigation scene with an admitted graph.
2. Author lighting and click **Apply game lighting**.
3. Enter an absolute `.mesh-lighting-preset.json` path in **Lighting preset file**, under an existing directory.
4. Optionally select **Include post-processing in preset**.
5. Click **Save applied lighting preset**.

Save captures valid applied lighting, including applied history not yet saved to the scene. It never captures unapplied text, applies defaults, changes history or saves the scene. The file must be new; existing files, links and directories are refused without replacement. Use a new filename for a revision.

The checkbox defaults off, preserving existing lighting-only presets. When selected it also captures post-processing enablement, bloom enablement, threshold, strength, radius and iterations. Capturing a disabled/default effect explicitly replaces destination effects with that disabled state. The checkbox controls Save; Load follows the file's format. Changing it is not a lighting edit.

## Stage a look in another scene

1. Open the target scene and apply its lighting if none is applied or saved.
2. Apply or discard its current form draft before loading.
3. Enter the preset path and click **Load lighting preset**.
4. Review the staged form, then **Apply game lighting**.
5. Use **Undo game lighting** or **Redo game lighting** if needed.
6. Use **File > Save mesh workspace** to persist the selected applied state.

Reusable values include direction, light/sky/ground colors, intensity, ambient, shadow enablement, tone mapping and exposure. Effects-inclusive presets also stage the captured effect settings. Lighting-only presets preserve destination post-processing values and whether they were authored.

The destination's shadow center/radius always remain its own, including exact form strings. Loading changes only the draft, leaving applied rendering, history, Frame, scene and sidecar unchanged until explicit Apply. Unsaved applied history remains available. Play/workspace-save guards require resolving the new draft first.

Dirty forms, invalid lighting, stale scene/epoch/graph contexts and external sidecar conflicts refuse preset operations. Failed or corrupt loads preserve draft and history. Preset path entry itself is not a lighting edit.

## File contract

Schema 1 uses `ORR_MESH_LIGHTING_PRESET_V1` and retains its original bytes and behavior. It forbids `lighting.post_process`. Opt-in schema 2 uses `ORR_MESH_LIGHTING_PRESET_V2` and requires a nonnull, strictly shaped `lighting.post_process` object. Missing/unknown/duplicate fields, positional objects, invalid numbers and mismatched versions/identities reject. Existing lighting and effect validators own numeric policy, including disabled effects. The complete file is limited to16KiB.

No preset carries scene filename, graph revision, spatial shadow bounds, assets, paths, scripts, URLs or network dependencies. After Apply and workspace Save, the ordinary scene-local lighting sidecar is authoritative; the original preset is unnecessary.

File operations require bounded local paths through existing real directory ancestry and reject symlink/reparse paths. Save validates before new-only publication. Use trusted stable directories: hostile concurrent filesystem mutation and power-loss durability are not guaranteed.

## Verification

CPU acceptance covers format compatibility, bounds, explicit disabled effects, draft/history preservation, cross-scene Apply/Undo/Save/reopen and settings passed to the existing runtime policy. Run focused suites:

```sh
cargo test -p orr_editor --features mesh-mission --lib mesh_lighting_preset
cargo test -p orr_editor --features mesh-mission --lib mesh_lighting_panel::presets
```

Existing opt-in software-Vulkan acceptance remains separate:

```sh
WGPU_BACKEND=vulkan ORR_MESH_LIGHTING_PRESET_ACCEPTANCE=1 \
ORR_MESH_LIGHTING_PRESET_CAPTURE_DIR=/absolute/new/preset-evidence \
cargo test -p orr_editor --features mesh-mission --test mesh_lighting_preset_ui -- \
  --ignored --exact portable_lighting_preset_cross_scene_stage_apply_undo_save_and_fresh_reopen \
  --nocapture --test-threads=1
```

That legacy test retains eight viewport captures, a full-editor screenshot and receipt for staging, Apply/Undo/Redo, corrupt loads and reopen. Its software-rendered evidence is not hardware-GPU, OS-input, player or export acceptance. New CPU checks do not establish native-window/GPU/audio/video behavior. Known Windows publication failures remain preserved; a locked desktop is not unlocked or bypassed.
