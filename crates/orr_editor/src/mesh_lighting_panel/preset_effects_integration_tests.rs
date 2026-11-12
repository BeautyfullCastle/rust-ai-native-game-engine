//! CPU-only cross-scene acceptance. Widget actions use accesskit, not a native
//! window or GPU playtest. Existing mesh_export post-process tests cover bundles.
use super::*;
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};

struct State {
    editor: Editor,
    panel: Panel,
}
fn harness(state: State) -> Harness<'static, State> {
    let mut ui = Harness::builder()
        .with_size([900.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut State| {
                egui::ScrollArea::vertical().show(ui, |ui| state.panel.show(ui, &state.editor));
            },
            state,
        );
    ui.run_steps(3);
    ui.get_by_label("Mesh game lighting").click_accesskit();
    ui.run_steps(3);
    ui
}
fn click(ui: &mut Harness<'_, State>, label: &str) {
    ui.get_by_label(label).scroll_to_me();
    ui.run_steps(20);
    ui.get_by_label(label).click_accesskit();
    ui.run_steps(3);
}
fn path(ui: &mut Harness<'_, State>, path: &Path) {
    let expected = path.to_str().unwrap();
    ui.get_by_role_and_label(Role::TextInput, "Lighting preset file")
        .scroll_to_me();
    ui.run_steps(20);
    ui.get_by_role_and_label(Role::TextInput, "Lighting preset file")
        .focus();
    ui.run_steps(2);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.run_steps(2);
    ui.get_by_role_and_label(Role::TextInput, "Lighting preset file")
        .type_text(expected);
    ui.run_steps(3);
    assert_eq!(
        ui.get_by_role_and_label(Role::TextInput, "Lighting preset file")
            .value()
            .as_deref(),
        Some(expected),
        "preset file widget must contain the intended absolute path before Save/Load"
    );
}

fn prepare(scene: &Path) -> State {
    let editor = crate::mesh_workspace::mission::prepare(
        scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    State { editor, panel }
}

#[test]
fn opt_in_effects_preset_cross_scene_stages_applies_saves_and_reopens_for_cpu_policy() {
    for workspace_save in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let source_scene = temp.path().join("source.scene.yaml");
        let mut source = prepare(&source_scene);
        let effects = orr_render::PostProcessSettings {
            enabled: true,
            bloom: false,
            threshold: 2.25,
            strength: 0.75,
            radius: 7,
            iterations: 3,
        };
        source.panel.intensity = "3.5".into();
        source.panel.exposure = "1.75".into();
        source.panel.post_process = effects;
        source.panel.post_process_authored = true;
        source.panel.form_dirty = true;
        source.panel.apply(&source.editor);
        source.panel.save();
        assert!(source.panel.error.is_none(), "{:?}", source.panel.error);
        let source_document = source.panel.current_document().unwrap().clone();
        let source_checksum = source.editor.checksum();
        let source_bytes = std::fs::read(&source_scene).unwrap();
        let source_sidecar =
            std::fs::read(crate::mesh_lighting_store::sidecar_path(&source_scene).unwrap())
                .unwrap();
        let preset = temp.path().join("effects.mesh-lighting-preset.json");
        let mut ui = harness(source);
        path(&mut ui, &preset);
        click(&mut ui, "Include post-processing in preset");
        click(&mut ui, "Save applied lighting preset");
        assert!(
            ui.state().panel.error.is_none(),
            "{:?}",
            ui.state().panel.error
        );
        let exported = crate::mesh_lighting_preset::store::load(&preset).unwrap();
        assert!(exported.includes_post_process());
        assert_eq!(ui.state().panel.current_document(), Some(&source_document));
        assert_eq!(ui.state().editor.checksum(), source_checksum);
        assert_eq!(std::fs::read(&source_scene).unwrap(), source_bytes);
        assert_eq!(
            std::fs::read(crate::mesh_lighting_store::sidecar_path(&source_scene).unwrap())
                .unwrap(),
            source_sidecar
        );
        let target_scene = temp.path().join("target.scene.yaml");
        let mut target = prepare(&target_scene);
        target.panel.shadow_center = ["1.25".into(), "-0".into(), "2.5".into()];
        target.panel.shadow_radius = "18".into();
        target.panel.form_dirty = true;
        target.panel.apply(&target.editor);
        target.panel.save();
        target.panel.shadow_center = ["+1.250".into(), "-0.000".into(), " 2.500 ".into()];
        target.panel.shadow_radius = "18.0000".into();
        let bounds = (
            target.panel.shadow_center.clone(),
            target.panel.shadow_radius.clone(),
        );
        let old = target.panel.current_document().unwrap().clone();
        let history_len = target.panel.history.len();
        let cursor = target.panel.cursor;
        let checksum = target.editor.checksum();
        let scene_bytes = std::fs::read(&target_scene).unwrap();
        let sidecar_path = crate::mesh_lighting_store::sidecar_path(&target_scene).unwrap();
        let saved = std::fs::read(&sidecar_path).unwrap();
        let mut target = harness(target);
        path(&mut target, &preset);
        click(&mut target, "Load lighting preset");
        let staged = &target.state().panel;
        assert!(staged.error.is_none(), "{:?}", staged.error);
        assert!(staged.form_dirty && staged.post_process_authored);
        assert_eq!(staged.post_process, effects);
        assert_eq!(staged.current_document(), Some(&old));
        assert_eq!((staged.history.len(), staged.cursor), (history_len, cursor));
        assert_eq!(
            (staged.shadow_center.clone(), staged.shadow_radius.clone()),
            bounds
        );
        assert_eq!(std::fs::read(&sidecar_path).unwrap(), saved);
        click(&mut target, "Apply game lighting");
        let applied = target.state().panel.current_document().unwrap().clone();
        assert_eq!(applied.post_process_settings(), effects);
        assert_eq!(applied.intensity, source_document.intensity);
        assert_eq!(applied.exposure, source_document.exposure);
        assert_eq!(applied.shadow_center, old.shadow_center);
        assert_eq!(applied.shadow_radius, old.shadow_radius);
        assert_eq!(target.state().panel.history.len(), history_len + 1);
        target.state_mut().panel.undo();
        assert_eq!(target.state().panel.current_document(), Some(&old));
        target.state_mut().panel.redo();
        assert_eq!(target.state().panel.current_document(), Some(&applied));
        if workspace_save {
            let state = target.state_mut();
            assert!(state.panel.save_workspace_changes(&state.editor).unwrap());
        } else {
            click(&mut target, "Save game lighting");
        }
        assert!(!target.state().panel.dirty());
        assert_eq!(target.state().editor.checksum(), checksum);
        assert_eq!(std::fs::read(&target_scene).unwrap(), scene_bytes);
        let fresh =
            crate::mesh_workspace::prepare(&target_scene, crate::mesh_workspace::Intent::Open)
                .unwrap();
        let mut reopened = Panel::default();
        reopened.sync(&fresh);
        assert_eq!(reopened.current_document(), Some(&applied));
        let requested = reopened.applied_post_process(&fresh).unwrap();
        requested.validate().unwrap();
        assert_eq!(requested, effects);
        let mut policy = crate::mesh_lighting::post_process_runtime::RuntimePolicy::default();
        assert_eq!(
            policy.select(fresh.scene_load_epoch(), fresh.path(), requested),
            effects
        );
        assert_eq!(fresh.checksum(), checksum);
    }
}
