use super::*;

fn setup() -> (tempfile::TempDir, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("target.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.shadow_center = ["1.25".into(), "-0".into(), "2.5".into()];
    panel.shadow_radius = "18".into();
    panel.form_dirty = true;
    panel.apply(&editor);
    assert!(panel.error.is_none());
    // Legitimate clean strings need not be canonical. A preset must not touch them.
    panel.shadow_center = ["+1.250".into(), "-0.000".into(), " 2.500 ".into()];
    panel.shadow_radius = "18.0000".into();
    (directory, editor, panel)
}

type HistorySnapshot = (Vec<(u64, Option<Document>)>, usize, u64, Option<u64>);

fn history(panel: &Panel) -> HistorySnapshot {
    (
        panel
            .history
            .iter()
            .map(|entry| (entry.id, entry.document.clone()))
            .collect(),
        panel.cursor,
        panel.next_id,
        panel.saved_id,
    )
}

fn fields(panel: &Panel) -> (Vec<String>, bool, bool, bool) {
    let mut values = Vec::new();
    values.extend(panel.direction.clone());
    values.extend(panel.color.clone());
    values.push(panel.intensity.clone());
    values.extend(panel.sky.clone());
    values.extend(panel.ground.clone());
    values.push(panel.ambient.clone());
    values.extend(panel.shadow_center.clone());
    values.push(panel.shadow_radius.clone());
    values.push(panel.exposure.clone());
    (values, panel.shadows, panel.tonemap, panel.form_dirty)
}

fn write_preset(path: &Path) {
    let mut source = Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap();
    source.intensity = 3.5;
    source.exposure = 2.0;
    source.shadow_center = [90.0, 80.0, 70.0];
    source.shadow_radius = 120.0;
    store::save_new(
        path,
        &Preset::from_document(&source).unwrap().encode().unwrap(),
    )
    .unwrap();
}

#[test]
fn preset_stages_into_unsaved_applied_history_without_changing_bounds_or_applied_state() {
    let (directory, editor, mut panel) = setup();
    let path = directory.path().join("source.mesh-lighting-preset.json");
    write_preset(&path);
    panel.presets.path = path.display().to_string();
    assert!(panel.history_is_dirty());
    let old_history = history(&panel);
    let old_applied = panel.applied_lighting(&editor);
    let bounds = (panel.shadow_center.clone(), panel.shadow_radius.clone());
    let checksum = editor.checksum();
    let scene = editor.path().unwrap();
    let scene_bytes = std::fs::read(&scene).unwrap();
    panel.load_preset(&editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(panel.form_dirty);
    assert_eq!(panel.intensity, "3.5");
    assert_eq!(panel.exposure, "2");
    assert_eq!(
        (panel.shadow_center.clone(), panel.shadow_radius.clone()),
        bounds
    );
    assert_eq!(history(&panel), old_history);
    assert_eq!(panel.applied_lighting(&editor), old_applied);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert!(
        crate::mesh_lighting_store::sidecar_stamp(&scene)
            .unwrap()
            .is_none()
    );
    assert!(panel.workspace_save_blocker(&editor).is_some());
    assert!(panel.play_blocker(&editor).is_some());

    panel.apply(&editor);
    let applied = panel.current_document().unwrap().clone();
    assert_eq!(applied.intensity, 3.5);
    assert_eq!(applied.shadow_center, [1.25, -0.0, 2.5]);
    assert_eq!(applied.shadow_radius, 18.0);
    assert_eq!(panel.history.len(), old_history.0.len() + 1);
    panel.undo();
    assert_eq!(panel.applied_lighting(&editor), old_applied);
    panel.redo();
    assert_eq!(panel.current_document(), Some(&applied));
    assert!(panel.save_workspace_changes(&editor).unwrap());
    assert!(!panel.dirty());
    let mut reopened = Panel::default();
    reopened.sync(&editor);
    assert_eq!(reopened.current_document(), Some(&applied));
}

#[test]
fn preset_save_exports_applied_unsaved_history_without_persisting_or_overwriting() {
    let (directory, editor, mut panel) = setup();
    let path = directory.path().join("new.mesh-lighting-preset.json");
    panel.presets.path = path.display().to_string();
    let old_fields = fields(&panel);
    let old_history = history(&panel);
    let scene = editor.path().unwrap();
    panel.save_applied_preset(&editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    let bytes = std::fs::read(&path).unwrap();
    assert!(Preset::decode(&bytes).is_ok());
    assert_eq!(fields(&panel), old_fields);
    assert_eq!(history(&panel), old_history);
    assert!(
        crate::mesh_lighting_store::sidecar_stamp(&scene)
            .unwrap()
            .is_none()
    );
    panel.save_applied_preset(&editor);
    assert!(
        panel
            .error
            .as_deref()
            .unwrap()
            .starts_with("Save lighting preset failed:")
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(history(&panel), old_history);
    assert_eq!(fields(&panel), old_fields);
}

#[test]
fn dirty_drafts_and_stale_scene_epoch_graph_reject_load_and_save_without_mutation() {
    for blocked in ["draft", "stale", "scene", "epoch", "graph", "pending"] {
        let (directory, editor, mut panel) = setup();
        let path = directory.path().join("source.mesh-lighting-preset.json");
        write_preset(&path);
        panel.presets.path = path.display().to_string();
        match blocked {
            "draft" => {
                panel.intensity = "unfinished".into();
                panel.form_dirty = true;
            }
            "stale" => panel.context_stale = true,
            "scene" => panel.scene = Some(directory.path().join("other.scene.yaml")),
            "epoch" => panel.scene_epoch = Some(editor.scene_load_epoch().wrapping_add(1)),
            "graph" => panel.graph_revision = Some([255; 32]),
            "pending" => panel.document_pending = true,
            _ => unreachable!(),
        }
        let old_fields = fields(&panel);
        let old_history = history(&panel);
        panel.load_preset(&editor);
        assert!(panel.error.is_some(), "{blocked}");
        assert_eq!(fields(&panel), old_fields, "{blocked}");
        assert_eq!(history(&panel), old_history, "{blocked}");
        let output = directory.path().join("blocked.mesh-lighting-preset.json");
        panel.presets.path = output.display().to_string();
        panel.save_applied_preset(&editor);
        assert!(panel.error.is_some(), "{blocked}");
        assert!(!output.exists(), "{blocked}");
        assert_eq!(fields(&panel), old_fields, "{blocked}");
        assert_eq!(history(&panel), old_history, "{blocked}");
    }
}

#[test]
fn corrupt_or_missing_preset_and_sidecar_conflicts_preserve_every_draft_field_and_history() {
    for failure in ["corrupt", "missing", "sidecar"] {
        let (directory, editor, mut panel) = setup();
        let path = directory.path().join("source.mesh-lighting-preset.json");
        match failure {
            "corrupt" => std::fs::write(&path, b"{\"schema\":1,\"lighting\":{}}").unwrap(),
            "missing" => {}
            "sidecar" => {
                write_preset(&path);
                let sidecar =
                    crate::mesh_lighting_store::sidecar_path(&editor.path().unwrap()).unwrap();
                std::fs::write(sidecar, b"external conflict").unwrap();
            }
            _ => unreachable!(),
        }
        panel.presets.path = path.display().to_string();
        let old_fields = fields(&panel);
        let old_history = history(&panel);
        let old_applied = panel.applied_lighting(&editor);
        panel.load_preset(&editor);
        assert!(panel.error.is_some(), "{failure}");
        assert_eq!(fields(&panel), old_fields, "{failure}");
        assert_eq!(history(&panel), old_history, "{failure}");
        assert_eq!(panel.applied_lighting(&editor), old_applied, "{failure}");
    }
}

#[test]
fn unapplied_defaults_cannot_be_silently_exported_or_replaced() {
    let (directory, editor, mut panel) = setup();
    panel.reset_history(None, Some(0));
    let path = directory.path().join("source.mesh-lighting-preset.json");
    write_preset(&path);
    panel.presets.path = path.display().to_string();
    let old_fields = fields(&panel);
    panel.load_preset(&editor);
    assert!(panel.error.is_some());
    assert_eq!(fields(&panel), old_fields);
    assert!(panel.current_document().is_none());
    let output = directory.path().join("blocked.mesh-lighting-preset.json");
    panel.presets.path = output.display().to_string();
    panel.save_applied_preset(&editor);
    assert!(panel.error.is_some());
    assert!(!output.exists());
}
