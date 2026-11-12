//! CPU actual-App/widget evidence, not native OS input or target-player execution.
use super::*;
use crate::game::EditorGame;
#[cfg(feature = "mesh-export")]
use egui_kittest::kittest::NodeT;
use egui_kittest::{Harness, kittest::Queryable};
use std::fs;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "orr-export-guide-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn scene(&self) -> PathBuf {
        let path = self.0.join("source.scene.yaml");
        fs::write(
            &path,
            include_bytes!("../../../../scenes/physics_demo.scene.yaml"),
        )
        .unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn harness(editor: Editor) -> Harness<'static, EditorApp> {
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| EditorApp::new(editor, None));
    h.state_mut().editor.sync();
    h.run_steps(3);
    h
}
fn assert_no_window_actions(h: &Harness<'_, EditorApp>) {
    assert!(
        !h.output()
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .any(|v| matches!(
                v,
                egui::ViewportCommand::Close
                    | egui::ViewportCommand::CancelClose
                    | egui::ViewportCommand::Focus
            ))
    );
    assert!(h.state().ui.dialog.is_none());
}
#[test]
fn routes_do_not_invent_default_or_cli_editor_export() {
    for game in [EditorGame::PhysGame, EditorGame::Yard3D] {
        let route = export_guide::route(game);
        assert!(route.settings.is_none());
        assert!(route.instructions.contains("no fallback"));
    }
    let route = export_guide::route(EditorGame::Arena);
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        feature = "arena-project-export"
    ))]
    assert!(matches!(
        route.settings,
        Some(export_guide::Settings::Arena)
    ));
    #[cfg(not(all(
        target_os = "linux",
        target_arch = "x86_64",
        feature = "arena-project-export"
    )))]
    assert!(route.settings.is_none());
    assert!(route.target.contains("Linux x86-64 host and player only"));
    #[cfg(not(all(
        target_os = "linux",
        target_arch = "x86_64",
        feature = "arena-project-export"
    )))]
    assert!(route.instructions.contains("orr_export_arena"));
    assert!(route.limits.contains("executes that runtime"));
}
#[test]
fn actual_app_guide_is_explicit_and_passive_even_when_source_disappears() {
    let dir = Directory::new();
    let scene = dir.scene();
    let mut h = harness(Editor::open(&scene).unwrap());
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Timeline);
    h.get_by_label("Export guide").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::ExportGuide);
    h.get_by_label("Export guide").focus();
    h.run_steps(2);
    let focus = h.ctx.memory(|m| m.focused());
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let diagnostics = h.state().editor.diagnostics();
    let bytes = fs::read(&scene).unwrap();
    let hidden = scene.with_extension("hidden");
    fs::rename(&scene, &hidden).unwrap();
    for _ in 0..12 {
        h.run_steps(1);
    }
    assert_eq!(h.state().ui.bottom_tab, BottomTab::ExportGuide);
    assert_eq!(h.ctx.memory(|m| m.focused()), focus);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    let after = h.state().editor.diagnostics();
    assert_eq!(
        after.sync_erp_wait.total_samples,
        diagnostics.sync_erp_wait.total_samples
    );
    assert_eq!(
        after.async_request.total_samples,
        diagnostics.async_request.total_samples
    );
    assert_eq!(after.pending_requests, diagnostics.pending_requests);
    assert_eq!(fs::read(&hidden).unwrap(), bytes);
    assert!(!scene.exists());
    assert_no_window_actions(&h);
    fs::rename(hidden, scene).unwrap();
    h.get_by_label("Timeline").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Timeline);
}
#[test]
fn actual_app_dirty_edit_does_not_open_export_guide_or_save() {
    let dir = Directory::new();
    let scene = dir.scene();
    let mut h = harness(Editor::open(&scene).unwrap());
    let bytes = fs::read(&scene).unwrap();
    assert!(h.state_mut().editor.select_named("body_05"));
    assert!(h.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(2))
    ));
    h.run_steps(3);
    assert!(h.state().editor.is_dirty());
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Timeline);
    assert_eq!(fs::read(scene).unwrap(), bytes);
    assert_no_window_actions(&h);
}
#[cfg(feature = "collect-dodge")]
#[test]
fn cli_profile_route_is_not_an_editor_feature_claim() {
    let route = export_guide::route(EditorGame::CollectDodge);
    assert!(route.settings.is_none());
    assert!(
        route
            .instructions
            .contains("project-export and collect-dodge")
    );
    assert!(route.limits.contains("smoke verification"));
}
#[cfg(feature = "room-project")]
#[test]
fn room_route_obeys_host_and_feature_boundary() {
    let route = export_guide::route(EditorGame::RoomEscape);
    assert_eq!(
        route.settings.is_some(),
        cfg!(all(
            windows,
            target_arch = "x86_64",
            feature = "windows-export"
        ))
    );
    if !cfg!(all(
        windows,
        target_arch = "x86_64",
        feature = "windows-export"
    )) {
        assert!(
            route
                .target
                .contains("Windows x86-64 host and windows-export")
        );
        assert!(route.instructions.contains("orr_export_room"));
    }
}
#[cfg(feature = "mesh-navigation")]
#[test]
fn mesh_route_preserves_pe_target_and_compiled_feature_boundary() {
    let route = export_guide::route(EditorGame::MeshNavigation);
    assert!(route.target.contains("Windows x86-64 player (PE32+)"));
    assert_eq!(route.settings.is_some(), cfg!(feature = "mesh-export"));
    assert!(route.instructions.contains("mesh-export"));
    assert!(route.limits.contains("never executes"));
}
#[cfg(feature = "mesh-hazards")]
#[test]
fn hazard_route_never_falls_back_to_legacy_mesh_export() {
    let route = export_guide::route(EditorGame::MeshHazardsV1);
    assert_eq!(
        route.settings.is_some(),
        cfg!(feature = "mesh-hazards-export")
    );
    assert!(route.target.contains("ELF"));
    assert!(route.limits.contains("sidecars are refused"));
    #[cfg(feature = "mesh-hazards-export")]
    assert_eq!(route.settings, Some(export_guide::Settings::Hazards));
}
#[cfg(feature = "mesh-export")]
#[test]
fn actual_app_mesh_guide_opens_existing_settings_without_export_or_trust() {
    let dir = Directory::new();
    let scene = dir.0.join("mesh.scene.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/mesh_navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut h = harness(Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap());
    let bytes = fs::read(&scene).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    h.get_by_label("Export guide").click();
    h.run_steps(2);
    assert!(!h.state().mesh_export.open);
    h.get_by_label("Open export settings").click();
    h.run_steps(2);
    assert!(h.state().mesh_export.open);
    assert!(!h.state().mesh_export.busy());
    assert!(!h.state().mesh_export.trusted_runtime);
    h.get_by_label("Required: choose a trusted mesh player executable.");
    h.get_by_label("Required: enter a new output directory.");
    h.get_by_label(
        "Required: explicitly confirm runtime trust; format checks do not establish trust.",
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(scene).unwrap(), bytes);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::ExportGuide);
    assert_no_window_actions(&h);
}
#[cfg(feature = "mesh-hazards-export")]
#[test]
fn actual_app_hazard_guide_opens_only_hazard_settings_and_explains_inputs() {
    let dir = Directory::new();
    let scene = dir.0.join("hazard.scene.yaml");
    let editor = crate::mesh_hazards_creator::prepare(&scene, &Default::default(), true).unwrap();
    let mut h = harness(editor);
    let bytes = fs::read(&scene).unwrap();
    h.get_by_label("Export guide").click();
    h.run_steps(2);
    h.get_by_label("Open export settings").click();
    h.run_steps(2);
    assert!(h.state().mesh_hazards_export.open);
    assert!(!h.state().mesh_export.open);
    assert!(!h.state().mesh_hazards_export.busy());
    assert!(!h.state().mesh_hazards_export.trusted_runtime);
    h.get_by_label("Required: choose a trusted hazard player executable.");
    h.get_by_label("Required: enter a new output directory.");
    assert_eq!(fs::read(scene).unwrap(), bytes);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::ExportGuide);
    assert_no_window_actions(&h);
}

#[cfg(all(
    feature = "mesh-hazards-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod hazard_export_widget_tests {
    use super::*;
    use egui::{Key, Modifiers, accesskit::Role};
    use orr_render::camera3d::OrbitCamera;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const SOURCE_CAMERA: &str = "game.scene.yaml.mesh-camera.json";

    fn camera_editor(scene: &Path) -> Editor {
        let editor =
            crate::mesh_hazards_creator::prepare(scene, &Default::default(), true).unwrap();
        let frame = editor.snapshot().unwrap();
        let graph_revision = frame
            .predicted()
            .singleton::<orr_remote::mesh_navigation::MeshScenePin>()
            .graph_revision;
        let camera = crate::mesh_camera::Document::from_orbit(OrbitCamera::new(
            [1.0, 3.0, 5.0],
            0.4,
            0.3,
            12.0,
        ))
        .unwrap();
        let source_filename = scene.file_name().unwrap().to_str().unwrap();
        let bytes = camera.encode(source_filename, graph_revision).unwrap();
        fs::write(
            crate::mesh_camera_store::sidecar_path(scene).unwrap(),
            bytes,
        )
        .unwrap();
        editor
    }

    fn fill(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(1);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(1);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(2);
    }

    fn open_settings(h: &mut Harness<'_, EditorApp>, runtime: &Path, output: &Path) {
        h.get_by_label("Export guide").click();
        h.run_steps(2);
        h.get_by_label("Open export settings").click();
        h.run_steps(2);
        assert!(h.state().mesh_hazards_export.open);
        fill(
            h,
            "Hazard player executable",
            &runtime.display().to_string(),
        );
        fill(
            h,
            "New output directory (must not exist)",
            &output.display().to_string(),
        );
        h.get_by_label("I trust this self-built, QA-tested hazard player")
            .click();
        h.run_steps(2);
    }

    fn write_camera_runtime(path: &Path) {
        let runtime = crate::mesh_hazards_export::test_camera_runtime(
            crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64,
        );
        fs::write(path, runtime).unwrap();
    }

    fn wait_until_idle(h: &mut Harness<'_, EditorApp>) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while h.state().mesh_hazards_export.busy() {
            assert!(Instant::now() < deadline, "hazard export worker timed out");
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn actual_app_exports_schema2_camera_as_four_file_bundle() {
        let dir = Directory::new();
        let scene = dir.0.join("source.scene.yaml");
        let editor = camera_editor(&scene);
        let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
        let source_camera = fs::read(&sidecar).unwrap();
        let runtime = dir.0.join("camera-player");
        write_camera_runtime(&runtime);
        let output = dir.0.join("bundle");
        let mut h = harness(editor);
        open_settings(&mut h, &runtime, &output);

        h.get_by_label("Mesh game camera").click();
        h.run_steps(2);
        let camera_control = h.get_by_role_and_label(Role::TextInput, "Game camera target X");
        assert!(!camera_control.accesskit_node().is_disabled());
        let export = h.get_by_role_and_label(Role::Button, "Export hazard game");
        assert!(!export.accesskit_node().is_disabled());
        export.click_accesskit();
        h.run_steps(2);
        wait_until_idle(&mut h);
        assert!(
            h.state()
                .mesh_hazards_export
                .status
                .contains("Hazard export complete")
        );
        let bundle = crate::mesh_hazards_export::open_bundle(&output).unwrap();
        assert!(bundle.camera_sha256.is_some());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 4);
        let bundled_camera = fs::read(output.join(SOURCE_CAMERA)).unwrap();
        assert_ne!(
            bundled_camera, source_camera,
            "the bundle camera is rebound to its scene filename"
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(output.join(crate::mesh_hazards_export::MANIFEST_FILENAME)).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["schema"], 2);
        assert_eq!(manifest["camera"]["name"].as_str(), Some(SOURCE_CAMERA));
        assert!(manifest["camera"]["sha256"].is_string());
        assert_eq!(
            fs::read(output.join(SOURCE_CAMERA)).unwrap(),
            bundled_camera,
            "the authored camera remains in the published bundle"
        );
    }

    #[test]
    fn actual_app_disables_camera_controls_until_closed_busy_panel_is_polled() {
        let dir = Directory::new();
        let scene = dir.0.join("source.scene.yaml");
        let editor = camera_editor(&scene);
        let runtime = dir.0.join("camera-player");
        write_camera_runtime(&runtime);
        let output = dir.0.join("bundle");
        let mut h = harness(editor);
        open_settings(&mut h, &runtime, &output);
        h.get_by_label("Mesh game camera").click();
        h.run_steps(2);

        let release_worker = h.state_mut().mesh_hazards_export.hold_pending_for_test();
        h.run_steps(2);
        assert!(h.state().mesh_hazards_export.busy());
        assert!(
            h.get_by_role_and_label(Role::TextInput, "Game camera target X")
                .accesskit_node()
                .is_disabled()
        );

        h.get_by_label("Close window").click();
        h.run_steps(1);
        assert!(!h.state().mesh_hazards_export.open);
        assert!(h.state().mesh_hazards_export.busy());
        release_worker();
        wait_until_idle(&mut h);
        assert!(
            h.state()
                .mesh_hazards_export
                .status
                .contains("disconnected")
        );
        assert!(!output.exists());
    }

    #[test]
    fn actual_app_dirty_camera_blocks_hazard_export() {
        let dir = Directory::new();
        let scene = dir.0.join("game.scene.yaml");
        let editor = camera_editor(&scene);
        let runtime = dir.0.join("camera-player");
        write_camera_runtime(&runtime);
        let output = dir.0.join("bundle");
        let mut h = harness(editor);
        open_settings(&mut h, &runtime, &output);
        h.get_by_label("Mesh game camera").click();
        h.run_steps(2);
        fill(&mut h, "Game camera target X", "2.0");
        assert!(h.state().mesh_camera.dirty());
        let export = h.get_by_role_and_label(Role::Button, "Export hazard game");
        assert!(export.accesskit_node().is_disabled());
        assert!(!h.state().mesh_hazards_export.busy());
        let blocker = h.state().mesh_creation_authoring_blocker().unwrap();
        assert!(blocker.contains("camera"));
        assert!(h.query_all_by_label(&blocker).count() > 0);
        h.run_steps(2);
        assert!(!output.exists());
    }
}

#[cfg(feature = "mesh-export")]
#[test]
fn actual_app_play_does_not_allow_opening_export_settings_or_switch_tabs() {
    use orr_edit::{Op, Origin};
    use orr_fp::{FP, fp};
    use orr_navigation::{AgentProfile, mesh::StaticMeshGraph};
    use orr_remote::mesh_navigation::{AgentSpec, MeshScenePin, PIN_NAME, mesh_doc_from_yaml};

    let dir = Directory::new();
    let scene = dir.0.join("mesh.scene.yaml");
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let pin = MeshScenePin::new(
        &graph,
        AgentSpec {
            start: [fp!(0.25), FP::ZERO, fp!(0.25)],
            goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap();
    let mut document = mesh_doc_from_yaml(include_str!(
        "../../../../scenes/mesh_navigation_blank.scene.yaml"
    ))
    .unwrap();
    let value = document
        .types()
        .get(PIN_NAME)
        .unwrap()
        .read(orr_reflect::bytemuck::bytes_of(&pin));
    document
        .apply(
            Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: String::new(),
                value,
            },
            Origin::User,
        )
        .unwrap();
    fs::write(&scene, document.save_yaml()).unwrap();
    let mut h = harness(Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap());
    assert!(h.state_mut().editor.start_play());
    h.state_mut().editor.sync();
    h.get_by_label("Export guide").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(
        h.get_by_label("Open export settings")
            .accesskit_node()
            .is_disabled()
    );
    assert!(!h.state().mesh_export.open);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::ExportGuide);
    assert_no_window_actions(&h);
}
