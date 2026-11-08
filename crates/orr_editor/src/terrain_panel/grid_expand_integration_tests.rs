//! CPU widget/persistence/navigation acceptance; no native-window or GPU claim.
#![allow(clippy::disallowed_types)]
use crate::{editor::Editor, game::EditorGame, model::Mode, terrain_panel::TerrainPanel};
use egui::accesskit::{Action, ActionData, ActionRequest, Role};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn settle(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.yard_rows_coherent()
            && (editor.mode() != Mode::Edit || editor.can_mutate())
            && editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == (editor.mode() == Mode::Play))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "fixture snapshot incoherent: {:?}",
            editor.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn fixture() -> (tempfile::TempDir, PathBuf, Editor, TerrainPanel) {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("grid-expand.yaml");
    #[cfg(feature = "navigation")]
    let (yaml, game) = (
        include_str!("../../../../scenes/navigation_blank.scene.yaml"),
        EditorGame::NavigationYard3D,
    );
    #[cfg(not(feature = "navigation"))]
    let (yaml, game) = (
        include_str!("../../../../scenes/yard3d_authoring.scene.yaml"),
        EditorGame::Yard3D,
    );
    fs::write(&scene, yaml).unwrap();
    let mut editor = Editor::open_game(&scene, game).unwrap();
    settle(&mut editor);
    let mut panel = TerrainPanel {
        asset_id: "terrain/grid-expand.orrt".into(),
        dimensions: [4, 3],
        origin: ["-2".into(), "1".into()],
        spacing: "0.5".into(),
        ..Default::default()
    };
    panel.create_for_editor(&editor).unwrap();
    let mut edits = Vec::new();
    for z in 0..3 {
        for x in 0..4 {
            edits.push(Edit::SetHeight {
                x,
                z,
                height: FP::from_raw((i64::from(x) * 3 - i64::from(z) * 2) * 8192),
            });
        }
    }
    edits.extend([
        Edit::SetHole {
            x: 1,
            z: 0,
            hole: true,
        },
        Edit::SetHole {
            x: 2,
            z: 1,
            hole: true,
        },
    ]);
    panel.document.apply(&edits).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    assert!(editor.save());
    settle(&mut editor); // Save schedules state/history refreshes.
    panel.grid_expand.dimensions = [7, 6];
    panel.grid_expand.fill = "0.5".into();
    (dir, scene, editor, panel)
}

fn click(h: &mut Harness<'_, (TerrainPanel, Editor)>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(h: &mut Harness<'_, (TerrainPanel, Editor)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn number(h: &mut Harness<'_, (TerrainPanel, Editor)>, label: &str, value: u32) {
    let node = h.get_by_label(label);
    let (target_node, target_tree) = node.accesskit_node().locate();
    h.event(egui::Event::AccessKitActionRequest(ActionRequest {
        target_node,
        target_tree,
        action: Action::SetValue,
        data: Some(ActionData::NumericValue(f64::from(value))),
    }));
    h.run_steps(3);
}

fn assert_growth(old: &Terrain, new: &Terrain) {
    assert_eq!((new.width(), new.depth()), (7, 6));
    assert_eq!(new.asset_id(), old.asset_id());
    assert_eq!(new.origin(), old.origin());
    assert_eq!(new.spacing(), old.spacing());
    for z in 0..6 {
        for x in 0..7 {
            let actual = new.heights()[(z * 7 + x) as usize];
            if x < 4 && z < 3 {
                assert_eq!(
                    actual,
                    old.heights()[(z * 4 + x) as usize],
                    "old vertex {x},{z}"
                );
                assert_eq!(
                    new.vertex_position(z * 7 + x),
                    old.vertex_position(z * 4 + x)
                );
            } else {
                assert_eq!(actual, FP::from_raw(32768), "new vertex {x},{z}");
            }
        }
    }
    for z in 0..5 {
        for x in 0..6 {
            let expected = x < 3 && z < 2 && old.holes()[(z * 3 + x) as usize];
            assert_eq!(new.holes()[(z * 6 + x) as usize], expected, "cell {x},{z}");
        }
    }
    assert_eq!(
        new.triangles().len(),
        56,
        "new cells are solid; two old holes survive stride remapping"
    );
    assert_eq!(
        new.vertex_position(41),
        Some([FP::ONE, FP::from_raw(32768), FP::from_raw(229376)])
    );
    let x = FP::from_raw(16384);
    let z = FP::from_raw(180224);
    assert!(old.sample(x, z).is_none());
    assert_eq!(
        new.sample(x, z),
        Some(FP::from_raw(32768)),
        "new +X/+Z area is queryable"
    );
}

#[test]
fn actual_grid_expansion_widgets_preserve_old_grid_one_history_step_save_and_navigation_new_area() {
    let (dir, scene, editor, mut panel) = fixture();
    // Targets and fill are edited through the real numeric/text widgets below.
    panel.grid_expand.dimensions = [4, 3];
    panel.grid_expand.fill = "0".into();
    let original = panel.document.terrain().unwrap().clone();
    let before = panel.document.bytes().unwrap().to_vec();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let selected = editor.selected_guid();
    let dirty = editor.is_dirty();
    let mut h = Harness::builder()
        .with_size([1400.0, 4200.0])
        .build_ui_state(
            |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
            (panel, editor),
        );
    h.run_steps(3);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Expand terrain grid");
    number(&mut h, "Grid expansion target width", 7);
    number(&mut h, "Grid expansion target depth", 6);
    text(&mut h, "Grid expansion fill height", "0.5");
    assert_eq!(h.state().0.grid_expand.dimensions, [7, 6]);
    assert_eq!(
        h.state().0.document.bytes(),
        Some(before.as_slice()),
        "draft is read-only"
    );
    assert!(!h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Apply grid expansion");
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    assert_growth(&original, panel.document.terrain().unwrap());
    assert_eq!(panel.dimensions, [7, 6]);
    assert!(panel.document.dirty() && panel.document.can_undo());
    let grown = panel.document.bytes().unwrap().to_vec();
    assert_eq!(
        fs::read(dir.path().join("terrain.orrt")).unwrap(),
        before,
        "Save is explicit"
    );
    assert_eq!(editor.history(), &history);
    assert_eq!(
        editor.checksum(),
        checksum,
        "terrain authoring cannot mutate the host Frame"
    );
    assert_eq!(editor.selected_guid(), selected);
    assert_eq!(editor.is_dirty(), dirty);
    #[cfg(feature = "navigation")]
    {
        let (panel, editor) = h.state_mut();
        let mut nav = crate::navigation_panel::NavigationPanel::default();
        nav.start = ["0".into(), "2.5".into()];
        nav.goal = ["1".into(), "3.5".into()];
        assert!(nav
            .build_route(editor, panel)
            .unwrap_err()
            .contains("Save terrain"));
        assert_eq!(editor.history(), &history);
        assert_eq!(editor.checksum(), checksum);
    }
    h.state_mut().0.selected_vertex = [6, 5];
    h.state_mut().0.selected_cell = [5, 4];
    click(&mut h, "Terrain Undo");
    let panel = &h.state().0;
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(
        !panel.document.can_undo(),
        "entire expansion is one history entry"
    );
    assert!(!panel.document.dirty());
    assert_eq!(panel.dimensions, [4, 3]);
    assert_eq!(panel.grid_expand.dimensions, [4, 3]);
    assert_eq!(panel.selected_vertex, [3, 2]);
    assert_eq!(panel.selected_cell, [2, 1]);
    assert_eq!(
        panel.height,
        crate::terrain_document::format_fixed(original.heights()[11])
    );
    assert!(panel.hole, "clamped cell refresh reads its old hole");
    click(&mut h, "Terrain Redo");
    assert_eq!(h.state().0.document.bytes(), Some(grown.as_slice()));
    assert_eq!(h.state().0.dimensions, [7, 6]);
    assert_eq!(h.state().0.grid_expand.dimensions, [7, 6]);
    assert!(!h.state().0.document.can_redo());
    click(&mut h, "Save terrain");
    let (panel, editor) = h.state_mut();
    panel.open_for_editor(editor).unwrap();
    assert_eq!(panel.document.bytes(), Some(grown.as_slice()));
    assert_growth(&original, panel.document.terrain().unwrap());
    assert!(!panel.document.dirty());
    assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), grown);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    #[cfg(feature = "navigation")]
    {
        let mut nav = crate::navigation_panel::NavigationPanel::default();
        nav.start = ["0".into(), "2.5".into()];
        nav.goal = ["1".into(), "3.5".into()];
        nav.max_slope = "4".into();
        nav.build_route(editor, panel).unwrap();
        settle(editor);
        let admitted = orr_remote::navigation_yard3d::navigation_from_view(
            editor.snapshot().unwrap().predicted(),
        )
        .unwrap();
        assert_eq!(
            admitted.terrain.cook(),
            grown,
            "explicit Build admits the saved expansion"
        );
        assert!(admitted
            .graph
            .vertices()
            .iter()
            .any(|v| *v == [FP::ONE, FP::from_raw(32768), FP::from_raw(229376)]));
        assert!(nav.play_blocker(editor, panel).is_none());
        assert!(editor.save());
        settle(editor);
        assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), grown);
    }
}

#[test]
fn invalid_expansion_and_render_admission_preserve_cache_history_and_noop_redo() {
    let (dir, scene, editor, mut panel) = fixture();
    panel
        .document
        .set_query(Some([FP::from_raw(-114688), FP::from_raw(114688)]))
        .unwrap();
    let before = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    let model = Arc::clone(panel.document.model().unwrap());
    let surface = panel.document.surface();
    let marker = panel.document.marker();
    let query = panel.document.query();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let scene_bytes = fs::read(&scene).unwrap();
    for cause in [
        "shrink-x",
        "shrink-z",
        "oversize",
        "integer-max",
        "parse",
        "fp-overflow",
        "display-range",
        "display-precision",
        "admission",
    ] {
        panel.grid_expand.dimensions = [7, 6];
        panel.grid_expand.fill = "0.5".into();
        panel.document.set_render_admission_error(None);
        match cause {
            "shrink-x" => panel.grid_expand.dimensions = [3, 6],
            "shrink-z" => panel.grid_expand.dimensions = [7, 2],
            "oversize" => panel.grid_expand.dimensions = [130, 6],
            "integer-max" => panel.grid_expand.dimensions = [u32::MAX, 6],
            "parse" => panel.grid_expand.fill = "invalid".into(),
            "fp-overflow" => panel.grid_expand.fill = "140737488355328".into(),
            "display-range" => panel.grid_expand.fill = "1000001".into(),
            "display-precision" => panel.grid_expand.fill = "256.0000152587890625".into(),
            "admission" => panel.document.set_render_admission_error(Some(
                "aggregate renderer capacity reserved by other content".into(),
            )),
            _ => unreachable!(),
        }
        assert!(panel.apply_grid_expand(&editor).is_err(), "{cause}");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()), "{cause}");
        assert_eq!(panel.document.revision(), revision);
        assert!(Arc::ptr_eq(panel.document.model().unwrap(), &model));
        assert_eq!(panel.document.query(), query);
        assert_eq!(panel.document.surface(), surface);
        assert_eq!(panel.document.marker(), marker);
        assert!(
            !panel.document.can_undo() && !panel.document.can_redo() && !panel.document.dirty()
        );
    }
    panel.document.set_render_admission_error(None);
    panel.grid_expand.dimensions = [7, 6];
    panel.grid_expand.fill = "0.5".into();
    panel.apply_grid_expand(&editor).unwrap();
    assert!(panel.undo(&editor).unwrap());
    let redo_revision = panel.document.revision();
    let redo_model = Arc::clone(panel.document.model().unwrap());
    panel.grid_expand.dimensions = [4, 3];
    panel.grid_expand.fill = "1000001".into();
    panel.apply_grid_expand(&editor).unwrap();
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(panel.document.revision(), redo_revision);
    assert!(Arc::ptr_eq(panel.document.model().unwrap(), &redo_model));
    assert!(panel.document.can_redo() && !panel.document.can_undo() && !panel.document.dirty());
    assert!(panel.redo(&editor).unwrap());
    assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), before);
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    // FP endpoint overflow is a core-only boundary: the editor rejects such
    // extreme coordinates during display admission before they can be attached.
    let terrain = Terrain::new(
        "endpoint-overflow".into(),
        2,
        2,
        [FP::from_raw(i64::MAX - 65536), FP::ZERO],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    let cooked = terrain.cook();
    let mut core = orr_terrain::TerrainDocument::new(terrain);
    assert!(core
        .expand_grid(3, 2, FP::ZERO)
        .unwrap_err()
        .to_string()
        .contains("overflow"));
    assert_eq!(core.terrain().cook(), cooked);
    assert!(!core.can_undo() && !core.can_redo());
    assert_eq!(core.dirty_region(), None);
}

#[test]
fn play_stale_scene_and_installed_read_only_terrain_reject_grid_expansion() {
    for cause in ["play", "stale", "package"] {
        let (dir, scene, mut editor, mut panel) = fixture();
        let local = fs::read(dir.path().join("terrain.orrt")).unwrap();
        let mut package = None;
        match cause {
            "play" => {
                #[cfg(feature = "navigation")]
                {
                    let mut nav = crate::navigation_panel::NavigationPanel::default();
                    nav.start = ["-1.75".into(), "1.25".into()];
                    nav.goal = ["-1.75".into(), "1.75".into()];
                    nav.max_slope = "4".into();
                    nav.build_route(&mut editor, &panel).unwrap();
                    settle(&mut editor);
                }
                assert!(editor.start_play(), "{:?}", editor.status());
                settle(&mut editor);
            }
            "stale" => {
                let next = dir.path().join("other.yaml");
                fs::copy(&scene, &next).unwrap();
                // Pinned navigation hosts do not support open_path. Preserve the
                // old panel and use a new host for the stale-association check.
                editor = Editor::open_game(&next, editor.game()).unwrap();
                settle(&mut editor);
                assert!(!panel.scene_matches(&editor));
            }
            "package" => {
                use orr_terrain_view::package::{install_and_reload, ASSET_PATH, PACKAGE_NAME};
                let project = dir.path().join("project");
                let installed = install_and_reload(
                    panel.document.terrain().unwrap(),
                    &project,
                    &dir.path().join("package-source"),
                    "1.0.0",
                )
                .unwrap();
                let path = project
                    .join(".orr/packages/objects")
                    .join(installed.package_digest)
                    .join(ASSET_PATH);
                let bytes = fs::read(&path).unwrap();
                panel
                    .document
                    .open_package(&project, PACKAGE_NAME, ASSET_PATH)
                    .unwrap();
                assert!(panel.document.read_only());
                package = Some((path, bytes));
            }
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let revision = panel.document.revision();
        let history = editor.history().clone();
        let scene_bytes = fs::read(&scene).unwrap();
        let model = Arc::clone(panel.document.model().unwrap());
        assert!(panel.apply_grid_expand(&editor).is_err(), "{cause}");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(Arc::ptr_eq(panel.document.model().unwrap(), &model));
        assert!(
            !panel.document.can_undo() && !panel.document.can_redo() && !panel.document.dirty()
        );
        assert_eq!(editor.history(), &history);
        assert_eq!(fs::read(scene).unwrap(), scene_bytes);
        assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), local);
        if let Some((path, bytes)) = package {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}
