//! Real CPU App renderer selection and egui authoring; no window or GPU output.
#![cfg(feature = "mesh-mission")]
use super::*;
use crate::{editor::Editor, game::EditorGame, mesh_lighting::Document};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::path::PathBuf;

fn games() -> Vec<EditorGame> {
    let games = vec![EditorGame::MeshNavigation];
    #[cfg(feature = "mesh-hazards")]
    let games = {
        let mut games = games;
        games.push(EditorGame::MeshHazardsV1);
        games
    };
    games
}
fn coherent(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate() && editor.yard_rows_coherent() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "incoherent Mesh fixture"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    scene: PathBuf,
    sidecar: PathBuf,
    app: EditorApp,
    saved: orr_render::PointLightSettings,
    checksum: u64,
    history: crate::model::History,
    files: [Vec<u8>; 2],
}
impl Fixture {
    fn new(game: EditorGame) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("preview.scene.yaml");
        let editor = crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        #[cfg(feature = "mesh-hazards")]
        let editor = if game.is_mesh_hazards() {
            let yaml = std::fs::read_to_string(&scene).unwrap();
            let mut doc =
                orr_remote::mesh_navigation::hazards::hazards_doc_from_yaml(&yaml).unwrap();
            std::fs::write(&scene, doc.save_yaml()).unwrap();
            Editor::open_game(&scene, game).unwrap()
        } else {
            editor
        };
        let mut editor = editor;
        coherent(&mut editor);
        assert_eq!(editor.game(), game);
        let revision =
            orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .graph
                .revision();
        let mut doc = Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap();
        doc.point_light = Some(orr_render::PointLight {
            position: [0.0, 4.0, 0.0],
            color: [1.0, 0.9, 0.7],
            intensity: 3.0,
            range: 12.0,
        });
        crate::mesh_lighting_store::save(&scene, Some(&doc), None, revision).unwrap();
        let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
        let files = [
            std::fs::read(&scene).unwrap(),
            std::fs::read(&sidecar).unwrap(),
        ];
        let checksum = editor.checksum();
        let history = editor.history().clone();
        let mut app = EditorApp::new(editor, None);
        app.mesh_lighting.sync(&app.editor);
        Self {
            _temp: temp,
            scene,
            sidecar,
            app,
            saved: doc.point_light_settings().unwrap(),
            checksum,
            history,
            files,
        }
    }
    fn unchanged(&self) {
        assert_eq!(self.app.editor.checksum(), self.checksum);
        assert_eq!(self.app.editor.history(), &self.history);
        self.files_unchanged();
    }
    fn files_unchanged(&self) {
        assert_eq!(std::fs::read(&self.scene).unwrap(), self.files[0]);
        assert_eq!(std::fs::read(&self.sidecar).unwrap(), self.files[1]);
    }
    fn preview(&mut self, enabled: bool) {
        self.app
            .mesh_lighting
            .set_point_light_preview(&self.app.editor, enabled)
            .unwrap();
    }
    fn sync_hidden(&mut self, events: Vec<egui::Event>) {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                focused: true,
                events,
                ..Default::default()
            },
            |ui| self.app.sync_mesh_light_preview(ui.ctx()),
        );
        output.textures_delta.clear();
    }
}
fn harness(f: Fixture) -> Harness<'static, Fixture> {
    let mut h = Harness::builder()
        .with_size([1400.0, 2800.0])
        .build_ui_state(
            |ui, f: &mut Fixture| {
                f.app.sync_mesh_light_preview(ui.ctx());
                f.app
                    .mesh_lighting
                    .show_with_navigation(ui, &f.app.editor, &f.app.mesh_navigation);
            },
            f,
        );
    h.run_steps(3);
    h.get_by_label("Mesh game lighting").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Point light settings").click_accesskit();
    h.run_steps(3);
    h
}
fn numeric(h: &mut Harness<'_, Fixture>, label: &str, value: f64) {
    let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
    h.event(egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::SetValue,
            data: Some(egui::accesskit::ActionData::NumericValue(value)),
        },
    ));
    h.run_steps(3);
}
fn text(h: &mut Harness<'_, Fixture>, label: &str, value: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn draft(h: &mut Harness<'_, Fixture>) -> orr_render::PointLightSettings {
    for (label, value) in [
        ("Point light position X", 2.0),
        ("Point light position Y", 6.0),
        ("Point light position Z", -1.0),
        ("Point light linear R", 0.25),
        ("Point light linear G", 0.5),
        ("Point light linear B", 0.75),
        ("Point light intensity", 7.0),
        ("Point light range", 9.0),
    ] {
        numeric(h, label, value);
    }
    orr_render::PointLightSettings {
        point_light: Some(orr_render::PointLight {
            position: [2.0, 6.0, -1.0],
            color: [0.25, 0.5, 0.75],
            intensity: 7.0,
            range: 9.0,
        }),
    }
}

#[test]
fn opt_in_renderer_selection_previews_every_point_field_and_toggle_off_reverts() {
    for game in games() {
        let mut h = harness(Fixture::new(game));
        let expected = draft(&mut h);
        let f = h.state_mut();
        assert!(!f.app.mesh_lighting.point_light_preview_enabled());
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), f.saved);
        f.preview(true);
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), expected);
        assert_eq!(
            f.app
                .mesh_lighting
                .applied_point_light_settings(&f.app.editor),
            Some(f.saved)
        );
        f.unchanged();
        f.preview(false);
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), f.saved);
        f.unchanged();
        f.preview(true);
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), expected);
        h.get_by_label("Enable point light").click_accesskit();
        h.run_steps(3);
        let f = h.state_mut();
        assert!(f.app.mesh_lighting.point_light_preview_enabled());
        assert_eq!(
            f.app.point_light_settings_for_frame().unwrap(),
            orr_render::PointLightSettings::default(),
            "a valid disabled draft must preview no light instead of falling back"
        );
        assert_eq!(
            f.app
                .mesh_lighting
                .applied_point_light_settings(&f.app.editor),
            Some(f.saved)
        );
        f.unchanged();
        let mut reopened = Editor::open_game(&f.scene, game).unwrap();
        coherent(&mut reopened);
        let mut restarted = EditorApp::new(reopened, None);
        restarted.mesh_lighting.sync(&restarted.editor);
        assert!(!restarted.mesh_lighting.point_light_preview_enabled());
        assert_eq!(restarted.point_light_settings_for_frame().unwrap(), f.saved);
        assert_eq!(restarted.editor.checksum(), f.checksum);
        f.unchanged();
    }
}

#[test]
fn invalid_form_retires_preview_and_valid_repair_requires_explicit_opt_in() {
    for game in games() {
        let mut h = harness(Fixture::new(game));
        let expected = draft(&mut h);
        h.state_mut().preview(true);
        assert_eq!(
            h.state_mut().app.point_light_settings_for_frame().unwrap(),
            expected
        );
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Ambient")
            .focus();
        h.run_steps(3);
        assert!(h.state().app.mesh_lighting.point_light_preview_enabled());
        assert_eq!(
            h.state_mut().app.point_light_settings_for_frame().unwrap(),
            expected,
            "text focus alone must preserve the live authoring preview"
        );
        text(&mut h, "Ambient", "invalid");
        let f = h.state_mut();
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), f.saved);
        assert!(!f.app.mesh_lighting.point_light_preview_enabled());
        assert!(f
            .app
            .mesh_lighting
            .set_point_light_preview(&f.app.editor, true)
            .is_err());
        f.unchanged();
        text(&mut h, "Ambient", "0.2");
        let f = h.state_mut();
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), f.saved);
        assert!(!f.app.mesh_lighting.point_light_preview_enabled());
        f.preview(true);
        assert_eq!(f.app.point_light_settings_for_frame().unwrap(), expected);
        f.unchanged();
    }
}

#[test]
fn scene_reload_mode_focus_and_dialog_retire_preview_without_automatic_revival() {
    for game in games() {
        for transition in ["scene", "host_reload", "mode", "focus", "dialog"] {
            let mut h = harness(Fixture::new(game));
            let expected = draft(&mut h);
            let f = h.state_mut();
            f.preview(true);
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), expected);
            match transition {
                "scene" => {
                    assert!(f.app.editor.open_path(&f.scene));
                    coherent(&mut f.app.editor);
                }
                "host_reload" => {
                    // Both reload routes advance the source epoch. Edit-state
                    // host epoch is zero; independent epoch pins have core tests.
                    let text = std::fs::read_to_string(&f.scene).unwrap();
                    f.app
                        .editor
                        .host_call("scene.load", serde_json::json!({ "text": text }))
                        .unwrap();
                    coherent(&mut f.app.editor);
                }
                "mode" => {
                    f.app.editor.step(0);
                    f.app.editor.sync();
                    assert_eq!(f.app.editor.mode(), crate::model::Mode::Play);
                }
                "focus" => f.sync_hidden(vec![
                    egui::Event::WindowFocused(false),
                    egui::Event::WindowFocused(true),
                ]),
                "dialog" => {
                    f.app.ui.dialog = Some(crate::app::Dialog {
                        kind: crate::app::DialogKind::Open,
                        text: f.scene.display().to_string(),
                    });
                }
                _ => unreachable!(),
            }
            // No inspector or viewport is drawn. App frame sync must retire
            // stale preview even before the renderer selector is queried.
            f.sync_hidden(vec![]);
            assert!(
                !f.app.mesh_lighting.point_light_preview_enabled(),
                "hidden-frame retirement: {transition}"
            );
            // Lifecycle calls above are deliberate fixture operations. Record
            // their resulting host state before checking the renderer selector.
            let checksum = f.app.editor.checksum();
            let history = f.app.editor.history().clone();
            let fallback = f
                .app
                .mesh_lighting
                .applied_point_light_settings(&f.app.editor)
                .unwrap_or_default();
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), fallback);
            assert!(
                !f.app.mesh_lighting.point_light_preview_enabled(),
                "{transition}"
            );
            assert_eq!(f.app.editor.checksum(), checksum);
            assert_eq!(f.app.editor.history(), &history);
            if transition == "mode" {
                f.app.editor.stop();
                coherent(&mut f.app.editor);
            }
            if transition == "dialog" {
                f.app.ui.dialog = None;
            }
            f.sync_hidden(vec![]);
            let checksum = f.app.editor.checksum();
            let history = f.app.editor.history().clone();
            let fallback = f
                .app
                .mesh_lighting
                .applied_point_light_settings(&f.app.editor)
                .unwrap_or_default();
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), fallback);
            assert!(!f.app.mesh_lighting.point_light_preview_enabled());
            assert_eq!(f.app.editor.checksum(), checksum);
            assert_eq!(f.app.editor.history(), &history);
            f.files_unchanged();
        }
    }
}
