//! Actual Yard host and CPU egui interaction; no GPU/bake quality claim.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{
    fs,
    time::{Duration, Instant},
};

const APPLY: &str = "Apply authored probe gradient";
fn settle(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        editor.sync();
        if editor.yard_rows_coherent()
            && editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == (editor.mode() == Mode::Play))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "matching Yard snapshot did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, Editor, IrradiancePanel) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("gradient.yaml");
    fs::write(
        &path,
        include_str!("../../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
    settle(&mut editor);
    let mut panel = IrradiancePanel::default();
    panel.open_for_editor(&editor, true).unwrap();
    panel.save().unwrap();
    assert_eq!(panel.gradient_axis, 0);
    assert_eq!(panel.gradient_start, [0.0; 3]);
    assert_eq!(panel.gradient_end, [1.0; 3]);
    (temp, path, editor, panel)
}

#[test]
fn actual_gradient_button_is_one_sidecar_undo_and_explicit_save_reopens() {
    let (_temp, path, editor, panel) = fixture();
    let scene = fs::read(&path).unwrap();
    let sidecar = panel.bindings.as_ref().unwrap().path.clone();
    let saved = fs::read(&sidecar).unwrap();
    let before = panel.bindings.as_ref().unwrap().document().clone();
    let revision = panel.bindings.as_ref().unwrap().revision();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let mut harness = Harness::builder()
        .with_size([1200.0, 1800.0])
        .build_ui_state(
            |ui, state: &mut (IrradiancePanel, Editor)| state.0.show(ui, &state.1),
            (panel, editor),
        );
    harness.run_steps(2);
    harness.get_by_label("Irradiance probes").click();
    harness.run_steps(3);
    harness.get_by_label(APPLY).click();
    harness.run_steps(3);
    let (panel, editor) = harness.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    let bindings = panel.bindings.as_ref().unwrap();
    let authored = bindings.document().clone();
    assert_ne!(authored, before);
    assert_eq!(bindings.revision(), revision + 1);
    assert!(bindings.dirty());
    assert_eq!(authored.grid.provenance, IrradianceProvenance::Authored);
    assert!(authored.bake_receipt.is_none());
    for z in 0..authored.grid.dimensions[2] {
        for y in 0..authored.grid.dimensions[1] {
            for x in 0..authored.grid.dimensions[0] {
                let value = x as f32 / (authored.grid.dimensions[0] - 1) as f32;
                let index = authored.grid.node_index([x, y, z]).unwrap();
                assert_eq!(
                    authored.grid.coefficients[index],
                    constant_irradiance([value; 3]).unwrap()
                );
            }
        }
    }
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    harness.get_by_label("Undo irradiance").click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().0.bindings.as_ref().unwrap().document(),
        &before
    );
    harness.get_by_label("Redo irradiance").click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().0.bindings.as_ref().unwrap().document(),
        &authored
    );
    harness.get_by_label("Save irradiance").click();
    harness.run_steps(3);
    assert!(!harness.state().0.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        IrradianceBindings::open(sidecar).unwrap().document(),
        &authored
    );
    assert_eq!(fs::read(path).unwrap(), scene);
    assert_eq!(harness.state().1.history(), &history);
    assert_eq!(harness.state().1.checksum(), checksum);
}

#[test]
fn invalid_gradient_endpoints_and_axis_are_transactional() {
    let (_temp, path, editor, mut panel) = fixture();
    let before = panel.bindings.as_ref().unwrap().document().clone();
    let revision = panel.bindings.as_ref().unwrap().revision();
    let scene = fs::read(&path).unwrap();
    let saved = fs::read(&panel.bindings.as_ref().unwrap().path).unwrap();
    let history = editor.history().clone();
    for endpoint in 0..2 {
        for channel in 0..3 {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1] {
                panel.gradient_start = [0.0; 3];
                panel.gradient_end = [1.0; 3];
                if endpoint == 0 {
                    panel.gradient_start[channel] = value;
                } else {
                    panel.gradient_end[channel] = value;
                }
                assert!(panel.apply_gradient(&editor).is_err());
                let b = panel.bindings.as_ref().unwrap();
                assert_eq!(b.document(), &before);
                assert_eq!(b.revision(), revision);
                assert!(!b.dirty());
                assert!(!b.can_undo());
            }
        }
    }
    panel.gradient_start = [0.0; 3];
    panel.gradient_end = [1.0; 3];
    panel.gradient_axis = 3;
    assert!(panel.apply_gradient(&editor).is_err());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &before);
    assert_eq!(panel.bindings.as_ref().unwrap().revision(), revision);
    assert_eq!(editor.history(), &history);
    assert_eq!(fs::read(path).unwrap(), scene);
    assert_eq!(
        fs::read(&panel.bindings.as_ref().unwrap().path).unwrap(),
        saved
    );
}

#[test]
fn play_and_changed_scene_reject_gradient_and_disable_actual_button() {
    for stale in [false, true] {
        let (temp, path, mut editor, mut panel) = fixture();
        let before = panel.bindings.as_ref().unwrap().document().clone();
        let revision = panel.bindings.as_ref().unwrap().revision();
        let saved = fs::read(&panel.bindings.as_ref().unwrap().path).unwrap();
        if stale {
            let other = temp.path().join("other.yaml");
            fs::copy(&path, &other).unwrap();
            assert!(editor.open_path(&other));
        } else {
            assert!(editor.start_play());
        }
        settle(&mut editor);
        panel.sync_for_editor(&editor);
        assert!(panel.apply_gradient(&editor).is_err());
        let mut harness = Harness::builder()
            .with_size([1200.0, 1800.0])
            .build_ui_state(
                |ui, state: &mut (IrradiancePanel, Editor)| state.0.show(ui, &state.1),
                (panel, editor),
            );
        harness.run_steps(2);
        harness.get_by_label("Irradiance probes").click();
        harness.run_steps(3);
        assert!(harness.get_by_label(APPLY).accesskit_node().is_disabled());
        harness.get_by_label(APPLY).click();
        harness.run_steps(3);
        let b = harness.state().0.bindings.as_ref().unwrap();
        assert_eq!(b.document(), &before);
        assert_eq!(b.revision(), revision);
        assert!(!b.dirty());
        assert_eq!(fs::read(&b.path).unwrap(), saved);
    }
}

#[test]
fn authored_gradient_clears_bake_receipt_and_undo_restores_exact_receipt() {
    let (_temp, _path, editor, mut panel) = fixture();
    let b = panel.bindings.as_mut().unwrap();
    let mut grid = b.document().grid.clone();
    grid.provenance = IrradianceProvenance::Baked;
    let receipt = irradiance_bindings::BakeReceipt {
        version: 1,
        algorithm_version: 1,
        fingerprint: "a".repeat(64),
        directions: 1024,
        max_triangle_tests: 32_000_000,
        max_node_visits: 64_000_000,
        max_duration_nanos: 10_000_000_000,
        triangle_count: 12,
        participant_count: 1,
        excluded_count: 2,
        snapshot_bytes: 2048,
        epsilon: orr_render::irradiance_bake::MIN_RAY_OFFSET as f32,
    };
    b.commit_baked(b.revision(), grid, receipt).unwrap();
    let baked = b.document().clone();
    let revision = b.revision();
    panel.apply_gradient(&editor).unwrap();
    let b = panel.bindings.as_ref().unwrap();
    assert_eq!(b.revision(), revision + 1);
    assert_eq!(b.document().version, 1);
    assert_eq!(b.document().grid.provenance, IrradianceProvenance::Authored);
    assert!(b.document().bake_receipt.is_none());
    panel.undo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &baked);
}
