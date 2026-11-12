use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_render::PostProcessSettings;

fn setup() -> (tempfile::TempDir, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("effects.scene.yaml");
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
    panel.shadow_center = ["+1.250".into(), "-0.000".into(), " 2.500 ".into()];
    panel.shadow_radius = "18.0000".into();
    (directory, editor, panel)
}
type History = (Vec<(u64, Option<Document>)>, usize, u64, Option<u64>);
fn history(p: &Panel) -> History {
    (
        p.history
            .iter()
            .map(|e| (e.id, e.document.clone()))
            .collect(),
        p.cursor,
        p.next_id,
        p.saved_id,
    )
}
fn appearance(p: &Panel) -> (Vec<String>, bool, bool, PostProcessSettings, bool, bool) {
    let mut fields = Vec::new();
    fields.extend(p.direction.clone());
    fields.extend(p.color.clone());
    fields.push(p.intensity.clone());
    fields.extend(p.sky.clone());
    fields.extend(p.ground.clone());
    fields.push(p.ambient.clone());
    fields.extend(p.shadow_center.clone());
    fields.push(p.shadow_radius.clone());
    fields.push(p.exposure.clone());
    (
        fields,
        p.shadows,
        p.tonemap,
        p.post_process,
        p.post_process_authored,
        p.form_dirty,
    )
}
fn effects() -> PostProcessSettings {
    PostProcessSettings {
        enabled: true,
        bloom: true,
        threshold: 2.5,
        strength: 0.7,
        radius: 6,
        iterations: 3,
    }
}
fn source() -> Document {
    let mut d = Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap();
    d.intensity = 3.5;
    d.shadow_center = [80.0, 90.0, 70.0];
    d.shadow_radius = 120.0;
    d.with_post_process(effects()).unwrap()
}
fn write(path: &Path, preset: Preset) {
    store::save_new(path, &preset.encode().unwrap()).unwrap();
}

#[test]
fn actual_include_checkbox_saves_explicit_disabled_effects_without_drafting_or_history_changes() {
    let (directory, editor, mut panel) = setup();
    let path = directory
        .path()
        .join("with-effects.mesh-lighting-preset.json");
    panel.presets.path = path.display().to_string();
    assert!(!panel.presets.include_post_process);
    assert!(!panel.current_document().unwrap().has_post_process());
    let old = appearance(&panel);
    let hist = history(&panel);
    let applied = panel.current_document().cloned();
    let scene = editor.path().unwrap();
    let scene_bytes = std::fs::read(&scene).unwrap();
    let mut h = Harness::builder()
        .with_size([1100.0, 600.0])
        .build_ui_state(
            |ui, state: &mut (Editor, Panel)| state.1.show_preset_controls(ui, &state.0),
            (editor, panel),
        );
    h.run_steps(3);
    h.get_by_label("Include post-processing in preset")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().1.presets.include_post_process);
    assert_eq!(appearance(&h.state().1), old);
    assert_eq!(history(&h.state().1), hist);
    assert_eq!(h.state().1.current_document(), applied.as_ref());
    h.get_by_label("Save applied lighting preset")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().1.error.is_none(), "{:?}", h.state().1.error);
    let preset = store::load(&path).unwrap();
    assert!(preset.includes_post_process());
    let target = source();
    let merged = preset.merge_into(&target).unwrap();
    assert!(merged.has_post_process());
    assert_eq!(
        merged.post_process_settings(),
        PostProcessSettings::default()
    );
    assert_eq!(appearance(&h.state().1), old);
    assert_eq!(history(&h.state().1), hist);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert!(crate::mesh_lighting_store::sidecar_stamp(&scene)
        .unwrap()
        .is_none());
}
#[test]
fn effect_preset_load_stages_complete_effects_preserves_bounds_and_uses_existing_apply_undo_save() {
    let (directory, editor, mut panel) = setup();
    let path = directory.path().join("source.mesh-lighting-preset.json");
    write(
        &path,
        Preset::from_document_with_post_process(&source()).unwrap(),
    );
    panel.presets.path = path.display().to_string();
    assert!(
        !panel.presets.include_post_process,
        "load must follow file contents, not Save checkbox"
    );
    let bounds = (panel.shadow_center.clone(), panel.shadow_radius.clone());
    let hist = history(&panel);
    let applied = panel.current_document().cloned();
    let scene = editor.path().unwrap();
    let scene_bytes = std::fs::read(&scene).unwrap();
    let mut h = Harness::builder()
        .with_size([1100.0, 600.0])
        .build_ui_state(
            |ui, state: &mut (Editor, Panel)| state.1.show_preset_controls(ui, &state.0),
            (editor, panel),
        );
    h.run_steps(3);
    h.get_by_label("Load lighting preset").click_accesskit();
    h.run_steps(3);
    let (editor, panel) = h.state_mut();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.post_process, effects());
    assert!(panel.post_process_authored);
    assert!(panel.form_dirty);
    assert_eq!(
        (panel.shadow_center.clone(), panel.shadow_radius.clone()),
        bounds
    );
    assert_eq!(history(panel), hist);
    assert_eq!(panel.current_document(), applied.as_ref());
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert!(crate::mesh_lighting_store::sidecar_stamp(&scene)
        .unwrap()
        .is_none());
    panel.apply(editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    let next = panel.current_document().unwrap().clone();
    assert_eq!(next.post_process_settings(), effects());
    assert!(next.has_post_process());
    assert_eq!(next.shadow_center, [1.25, -0.0, 2.5]);
    assert_eq!(next.shadow_radius, 18.0);
    panel.undo();
    assert_eq!(panel.current_document(), applied.as_ref());
    assert!(!panel.post_process_authored);
    panel.redo();
    assert_eq!(panel.current_document(), Some(&next));
    assert!(panel.save_workspace_changes(editor).unwrap());
    let mut reopened = Panel::default();
    reopened.sync(editor);
    assert_eq!(reopened.current_document(), Some(&next));
    assert_eq!(reopened.post_process, effects());
    assert!(reopened.post_process_authored);
}
#[test]
fn ordinary_preset_keeps_target_effect_values_and_authored_presence_with_save_option_enabled() {
    for authored in [false, true] {
        let (directory, editor, mut panel) = setup();
        if authored {
            panel.post_process = effects();
            panel.post_process_authored = true;
            panel.form_dirty = true;
            panel.apply(&editor);
            assert!(panel.error.is_none());
        }
        let current = panel.current_document().unwrap().clone();
        let path = directory.path().join("legacy.mesh-lighting-preset.json");
        let legacy = Preset::from_document(&source()).unwrap();
        assert!(!legacy.includes_post_process());
        write(&path, legacy);
        panel.presets.path = path.display().to_string();
        panel.presets.include_post_process = true;
        let old = panel.post_process;
        let hist = history(&panel);
        panel.load_preset(&editor);
        assert!(panel.error.is_none());
        assert_eq!(panel.post_process, old);
        assert_eq!(panel.post_process_authored, authored);
        assert_eq!(history(&panel), hist);
        assert_eq!(panel.current_document(), Some(&current));
        panel.apply(&editor);
        assert_eq!(
            panel.current_document().unwrap().has_post_process(),
            authored
        );
        assert_eq!(
            panel.current_document().unwrap().post_process_settings(),
            old
        );
    }
}
#[test]
fn rejected_effect_presets_and_dirty_or_stale_context_preserve_draft_history_and_files() {
    for blocked in ["corrupt", "dirty", "stale", "sidecar"] {
        let (directory, editor, mut panel) = setup();
        let path = directory.path().join("source.mesh-lighting-preset.json");
        if blocked == "corrupt" {
            std::fs::write(&path, b"{bad}").unwrap();
        } else {
            write(
                &path,
                Preset::from_document_with_post_process(&source()).unwrap(),
            );
        }
        panel.presets.path = path.display().to_string();
        panel.presets.include_post_process = true;
        match blocked {
            "dirty" => {
                panel.post_process.threshold = f32::INFINITY;
                panel.form_dirty = true;
            }
            "stale" => panel.context_stale = true,
            "sidecar" => {
                std::fs::write(
                    crate::mesh_lighting_store::sidecar_path(&editor.path().unwrap()).unwrap(),
                    b"external",
                )
                .unwrap();
            }
            _ => {}
        }
        let old = appearance(&panel);
        let hist = history(&panel);
        let bytes = std::fs::read(&path).unwrap();
        panel.load_preset(&editor);
        assert!(panel.error.is_some(), "{blocked}");
        assert_eq!(appearance(&panel), old);
        assert_eq!(history(&panel), hist);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        if blocked != "corrupt" {
            let output = directory
                .path()
                .join("must-not-publish.mesh-lighting-preset.json");
            panel.presets.path = output.display().to_string();
            panel.save_applied_preset(&editor);
            assert!(panel.error.is_some());
            assert!(!output.exists());
            assert_eq!(appearance(&panel), old);
            assert_eq!(history(&panel), hist);
        }
    }
}
