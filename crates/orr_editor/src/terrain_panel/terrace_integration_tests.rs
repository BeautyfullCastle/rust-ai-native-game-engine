//! Real CPU widgets, terrain persistence and admitted navigation; no GPU claim.
#![allow(clippy::disallowed_types)]
use super::Spec;
use crate::{editor::Editor, game::EditorGame, model::Mode, terrain_panel::TerrainPanel};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_fp::FP;
use orr_terrain::Edit;
use std::{
    fs,
    path::PathBuf,
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
        assert!(Instant::now() < deadline, "fixture snapshot incoherent");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn spec() -> Spec {
    Spec {
        min: [1, 1],
        max: [3, 3],
        base: "0".into(),
        step: "0.5".into(),
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, Editor, TerrainPanel) {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("terrace.yaml");
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
        dimensions: [7, 7],
        origin: ["0".into(), "0".into()],
        ..Default::default()
    };
    panel.create_for_editor(&editor).unwrap();
    let mut edits = Vec::new();
    for z in 0..7 {
        for x in 0..7 {
            edits.push(Edit::SetHeight {
                x,
                z,
                height: FP::from_raw(i64::from(x) * 24576),
            });
        }
    }
    edits.push(Edit::SetHeight {
        x: 3,
        z: 3,
        height: FP::from_raw(-24576),
    });
    edits.push(Edit::SetHole {
        x: 5,
        z: 5,
        hole: true,
    });
    panel.document.apply(&edits).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    assert!(editor.save());
    settle(&mut editor); // Save schedules state/history refreshes.
    panel.terrace = spec();
    (dir, scene, editor, panel)
}
fn click(h: &mut Harness<'_, (TerrainPanel, Editor)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(h: &mut Harness<'_, (TerrainPanel, Editor)>, label: &str, value: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}

#[test]
fn actual_terrace_widgets_single_undo_redo_explicit_save_reopen_and_navigation_use() {
    let (dir, scene, editor, mut panel) = fixture();
    panel.terrace.step = "1".into();
    let before = panel.document.bytes().unwrap().to_vec();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let selected = editor.selected_guid();
    let dirty = editor.is_dirty();
    let mut h = Harness::builder().with_size([1400., 3400.]).build_ui_state(
        |ui, s: &mut (TerrainPanel, Editor)| s.0.show_in_parent_scroll(ui, &s.1),
        (panel, editor),
    );
    h.run_steps(2);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Terrace terrain rectangle");
    text(&mut h, "Terrace positive step", "0.5");
    assert!(!h
        .get_by_label("Apply terrace rectangle")
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        h.state().0.document.bytes(),
        Some(before.as_slice()),
        "preview cannot author"
    );
    click(&mut h, "Apply terrace rectangle");
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    let t = panel.document.terrain().unwrap();
    for z in 0..7 {
        for x in 0..7 {
            let original = if x == 3 && z == 3 {
                -24576
            } else {
                i64::from(x) * 24576
            };
            let expected = if (1..=3).contains(&x) && (1..=3).contains(&z) {
                original.div_euclid(32768) * 32768
            } else {
                original
            };
            assert_eq!(
                t.heights()[(z * 7 + x) as usize].raw(),
                expected,
                "vertex{x},{z}"
            );
        }
    }
    assert!(t.holes()[35]);
    assert_eq!(t.holes().iter().filter(|v| **v).count(), 1);
    let changed = panel.document.bytes().unwrap().to_vec();
    assert_ne!(changed, before);
    assert!(panel.document.dirty());
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.selected_guid(), selected);
    assert_eq!(editor.is_dirty(), dirty);
    assert_eq!(
        fs::read(dir.path().join("terrain.orrt")).unwrap(),
        before,
        "Save is explicit"
    );
    click(&mut h, "Terrain Undo");
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    assert!(!h.state().0.document.can_undo());
    click(&mut h, "Terrain Redo");
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    assert!(!h.state().0.document.can_redo());
    click(&mut h, "Save terrain");
    let (panel, editor) = h.state_mut();
    panel.open_for_editor(editor).unwrap();
    assert_eq!(panel.document.bytes(), Some(changed.as_slice()));
    assert!(!panel.document.dirty());
    assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), changed);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    #[cfg(feature = "navigation")]
    {
        let mut nav = crate::navigation_panel::NavigationPanel::default();
        nav.start = ["1".into(), "2".into()];
        nav.goal = ["3".into(), "2".into()];
        nav.max_slope = "4".into();
        nav.build_route(editor, panel).unwrap();
        settle(editor);
        let admitted = orr_remote::navigation_yard3d::navigation_from_view(
            editor.snapshot().unwrap().predicted(),
        )
        .unwrap();
        assert_eq!(
            admitted.terrain.cook(),
            changed,
            "host admitted the saved terraced bytes"
        );
        assert!(admitted
            .graph
            .vertices()
            .iter()
            .any(|v| v[0] == FP::from_int(2)
                && v[2] == FP::from_int(2)
                && v[1] == FP::from_raw(32768)));
        assert!(editor.save());
        settle(editor);
        assert_eq!(
            fs::read(dir.path().join("terrain.orrt")).unwrap(),
            changed,
            "route authoring cannot rewrite terrain"
        );
    }
}

#[test]
fn invalid_late_overflow_step_bounds_and_noop_preserve_bytes_revision_history_and_dirty() {
    let (dir, scene, editor, mut panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let scene_bytes = fs::read(&scene).unwrap();
    for cause in [
        "late-overflow",
        "zero",
        "negative",
        "parse",
        "bounds",
        "reversed",
    ] {
        panel.terrace = spec();
        match cause {
            "late-overflow" => {
                panel.terrace.base = "-0.25".into();
                panel.terrace.step = "140737488355327.9999847412109375".into();
            }
            "zero" => panel.terrace.step = "0".into(),
            "negative" => panel.terrace.step = "-1".into(),
            "parse" => panel.terrace.base = "invalid".into(),
            "bounds" => panel.terrace.max = [7, 3],
            "reversed" => panel.terrace.min = [4, 1],
            _ => {}
        }
        let error = panel.apply_terrace(&editor).unwrap_err();
        if cause == "late-overflow" {
            assert!(error.contains("fixed-point range"), "{error}");
        }
        assert_eq!(panel.document.bytes(), Some(before.as_slice()), "{cause}");
        assert_eq!(panel.document.revision(), revision);
        assert!(!panel.document.dirty());
        assert!(!panel.document.can_undo());
        assert!(!panel.document.can_redo());
    }
    // Every source height is an exact multiple of one FP unit. A no-op must
    // preserve an existing redo entry, rather than open a fresh transaction.
    panel.terrace = spec();
    panel.apply_terrace(&editor).unwrap();
    panel.document.undo().unwrap();
    let redo_revision = panel.document.revision();
    let dirty = panel.document.dirty();
    panel.terrace.step = "0.0000152587890625".into();
    panel.apply_terrace(&editor).unwrap();
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(panel.document.revision(), redo_revision);
    assert_eq!(panel.document.dirty(), dirty);
    assert!(!panel.document.can_undo());
    assert!(panel.document.can_redo());
    panel.document.redo().unwrap();
    assert_ne!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), before);
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}

#[test]
fn play_stale_scene_and_installed_package_reject_terrace_without_writes() {
    for cause in ["play", "stale", "package"] {
        let (dir, scene, mut editor, mut panel) = fixture();
        let local = fs::read(dir.path().join("terrain.orrt")).unwrap();
        let mut package = None;
        match cause {
            "play" => {
                #[cfg(feature = "navigation")]
                {
                    let mut nav = crate::navigation_panel::NavigationPanel::default();
                    nav.start = ["1".into(), "2".into()];
                    nav.goal = ["3".into(), "2".into()];
                    nav.max_slope = "4".into();
                    nav.build_route(&mut editor, &panel).unwrap();
                    settle(&mut editor);
                }
                assert!(editor.start_play());
                settle(&mut editor);
            }
            "stale" => {
                let next = dir.path().join("other.yaml");
                fs::copy(&scene, &next).unwrap();
                assert!(editor.open_path(&next));
                settle(&mut editor);
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
                let object = project
                    .join(".orr/packages/objects")
                    .join(installed.package_digest)
                    .join(ASSET_PATH);
                let bytes = fs::read(&object).unwrap();
                panel
                    .document
                    .open_package(&project, PACKAGE_NAME, ASSET_PATH)
                    .unwrap();
                assert!(panel.document.read_only());
                package = Some((object, bytes));
            }
            _ => {}
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let revision = panel.document.revision();
        let history = editor.history().clone();
        let scene_bytes = fs::read(&scene).unwrap();
        assert!(panel.apply_terrace(&editor).is_err(), "{cause}");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(!panel.document.can_undo());
        assert!(!panel.document.can_redo());
        assert!(!panel.document.dirty());
        assert_eq!(editor.history(), &history);
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
        assert_eq!(fs::read(dir.path().join("terrain.orrt")).unwrap(), local);
        if let Some((path, bytes)) = package {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}
