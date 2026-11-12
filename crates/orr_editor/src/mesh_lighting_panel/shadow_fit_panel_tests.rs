use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::time::{Duration, Instant};
fn setup() -> (
    tempfile::TempDir,
    Editor,
    Panel,
    crate::mesh_navigation::Panel,
) {
    let root = tempfile::tempdir().unwrap();
    use orr_fp::{fp, FP};
    use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
    use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
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
    let mut document = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [fp!(0.25), FP::ZERO, fp!(0.25)],
            goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap();
    let scene = root.path().join("fit.scene.yaml");
    std::fs::write(&scene, document.save_yaml()).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    let mut panel = Panel::default();
    let mut navigation = crate::mesh_navigation::Panel::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        navigation.sync(&editor);
        panel.sync(&editor);
        if panel.can_edit(&editor) && navigation.workspace_save_blocker(&editor).is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fit fixture not admitted: {:?}",
            navigation.workspace_save_blocker(&editor)
        );
        std::thread::yield_now();
    }
    (root, editor, panel, navigation)
}
fn bounds(p: &Panel) -> ([String; 3], String) {
    (p.shadow_center.clone(), p.shadow_radius.clone())
}
fn other_fields(
    p: &Panel,
) -> (
    Vec<String>,
    bool,
    bool,
    orr_render::PostProcessSettings,
    bool,
) {
    let mut fields = Vec::new();
    fields.extend(p.direction.clone());
    fields.extend(p.color.clone());
    fields.push(p.intensity.clone());
    fields.extend(p.sky.clone());
    fields.extend(p.ground.clone());
    fields.push(p.ambient.clone());
    fields.push(p.exposure.clone());
    (
        fields,
        p.shadows,
        p.tonemap,
        p.post_process,
        p.post_process_authored,
    )
}
type HistorySnapshot = (Vec<(u64, Option<Document>)>, usize, u64, Option<u64>);
fn history(p: &Panel) -> HistorySnapshot {
    (
        p.history
            .iter()
            .map(|x| (x.id, x.document.clone()))
            .collect(),
        p.cursor,
        p.next_id,
        p.saved_id,
    )
}
#[test]
fn fit_button_only_stages_bounds_not_applied_lighting_history_or_files() {
    let (_root, editor, mut panel, navigation) = setup();
    panel.shadows = false;
    panel.shadow_center = ["100".into(), "100".into(), "100".into()];
    panel.shadow_radius = "1".into();
    panel.direction = ["-0.2".into(), "-0.9".into(), "-0.4".into()];
    let before_other = other_fields(&panel);
    let before_history = history(&panel);
    let before_bounds = bounds(&panel);
    let scene = editor.path().unwrap();
    let scene_bytes = std::fs::read(&scene).unwrap();
    let checksum = editor.checksum();
    let editor_history = editor.history().clone();
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    assert!(!sidecar.exists());
    let mut h = Harness::builder().with_size([900.0, 300.0]).build_ui_state(
        |ui, state: &mut (Editor, Panel, crate::mesh_navigation::Panel)| {
            state.1.show_shadow_fit(ui, &state.0, &state.2)
        },
        (editor, panel, navigation),
    );
    h.run_steps(2);
    h.get_by_label("Fit static shadow bounds").click_accesskit();
    h.run_steps(2);
    let (_, panel, _) = h.state();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(panel.form_dirty);
    assert_ne!(bounds(panel), before_bounds);
    assert_eq!(other_fields(panel), before_other);
    assert_eq!(history(panel), before_history);
    assert_eq!(h.state().0.checksum(), checksum);
    assert_eq!(h.state().0.history(), &editor_history);
    assert_eq!(std::fs::read(scene).unwrap(), scene_bytes);
    assert!(!sidecar.exists());
}
#[test]
fn invalid_draft_unready_navigation_and_stale_context_leave_four_strings_untouched() {
    let (_root, editor, mut panel, navigation) = setup();
    for invalid in 0..4 {
        panel.direction = ["-0.45".into(), "-0.8".into(), "-0.35".into()];
        panel.context_stale = false;
        panel.shadow_center = ["+1.000".into(), "-0.000".into(), " 2.0 ".into()];
        panel.shadow_radius = "18.000".into();
        if invalid == 0 {
            panel.direction[0] = "NaN".into();
        }
        if invalid == 1 {
            panel.direction = ["0".into(), "0".into(), "0".into()];
        }
        if invalid == 2 {
            panel.context_stale = true;
        }
        let before = bounds(&panel);
        let before_history = history(&panel);
        let unready = crate::mesh_navigation::Panel::default();
        assert!(panel
            .fit_shadow_bounds(&editor, if invalid == 3 { &unready } else { &navigation })
            .is_err());
        assert_eq!(bounds(&panel), before);
        assert_eq!(history(&panel), before_history);
    }
}
#[test]
fn play_and_wrong_scene_are_rejected_without_draft_mutation() {
    let (_root, mut editor, mut panel, navigation) = setup();
    let before = bounds(&panel);
    panel.scene = Some(PathBuf::from("unrelated.scene.yaml"));
    assert!(panel.fit_shadow_bounds(&editor, &navigation).is_err());
    assert_eq!(bounds(&panel), before);
    panel.scene = editor.path();
    editor.step(0);
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    assert!(panel.fit_shadow_bounds(&editor, &navigation).is_err());
    assert_eq!(bounds(&panel), before);
}
#[test]
fn fitted_draft_is_one_time_and_retained_after_direction_and_geometry_drafts() {
    let (_root, editor, mut panel, navigation) = setup();
    panel.fit_shadow_bounds(&editor, &navigation).unwrap();
    let fitted = bounds(&panel);
    let before_history = history(&panel);
    panel.direction = ["0.1".into(), "-1".into(), "0.1".into()];
    let mut later_geometry = crate::mesh_navigation::Panel::default();
    later_geometry.create_ramp_and_bridge().unwrap();
    assert!(later_geometry.has_unplaced_draft());
    panel.sync(&editor);
    assert_eq!(bounds(&panel), fitted);
    assert!(panel.fit_shadow_bounds(&editor, &later_geometry).is_err());
    assert_eq!(bounds(&panel), fitted);
    assert_eq!(history(&panel), before_history);
}
