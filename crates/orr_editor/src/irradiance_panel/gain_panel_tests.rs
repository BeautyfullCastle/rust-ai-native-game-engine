//! Real Yard host + CPU egui authoring; no GPU or traced-bake quality claim.
#![allow(clippy::disallowed_types)] // Bounded test host polling only.
use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::{
    fs,
    time::{Duration, Instant},
};
const APPLY: &str = "Apply irradiance RGB gain";
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
            "coherent Yard snapshot unavailable"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, Editor, IrradiancePanel) {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("gain.yaml");
    fs::write(
        &scene,
        include_str!("../../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    settle(&mut editor);
    let mut panel = IrradiancePanel::default();
    assert_eq!(panel.coefficient_gain, [1.0; 3]);
    panel.open_for_editor(&editor, true).unwrap();
    let b = panel.bindings.as_mut().unwrap();
    let mut grid = b.document().grid.clone();
    for node in &mut grid.coefficients {
        for (i, c) in node.iter_mut().enumerate() {
            *c = [(i as f32 + 1.0) * -0.125, 0.25, 0.5];
        }
    }
    b.replace_grid(grid).unwrap();
    panel.save().unwrap();
    let path = panel.bindings.as_ref().unwrap().path.clone();
    panel.bindings = Some(IrradianceBindings::open(path).unwrap());
    (temp, scene, editor, panel)
}
fn receipt() -> irradiance_bindings::BakeReceipt {
    irradiance_bindings::BakeReceipt {
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
    }
}
#[test]
fn actual_gain_button_is_single_undo_redo_and_explicit_save_reopens_exact_coefficients() {
    let (_temp, path, editor, mut panel) = fixture();
    panel.coefficient_gain = [2.0, 0.5, 0.0];
    let before = panel.bindings.as_ref().unwrap().document().clone();
    let revision = panel.bindings.as_ref().unwrap().revision();
    let sidecar = panel.bindings.as_ref().unwrap().path.clone();
    let saved = fs::read(&sidecar).unwrap();
    let scene = fs::read(&path).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let mut h = Harness::builder()
        .with_size([1200.0, 2200.0])
        .build_ui_state(
            |ui, s: &mut (IrradiancePanel, Editor)| s.0.show(ui, &s.1),
            (panel, editor),
        );
    h.run_steps(2);
    h.get_by_label("Irradiance probes").click();
    h.run_steps(3);
    h.get_by_label(APPLY).click();
    h.run_steps(3);
    let b = h.state().0.bindings.as_ref().unwrap();
    let after = b.document().clone();
    assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
    assert_eq!(b.revision(), revision + 1);
    assert!(b.dirty());
    let mut expected = before.grid.clone();
    expected.provenance = IrradianceProvenance::Authored;
    for node in &mut expected.coefficients {
        for c in node {
            c[0] *= 2.0;
            c[1] *= 0.5;
            c[2] *= 0.0;
        }
    }
    assert_eq!(after.grid, expected);
    assert!(after.bake_receipt.is_none());
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    h.get_by_label("Undo irradiance").click();
    h.run_steps(3);
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &before);
    assert!(!h.state().0.bindings.as_ref().unwrap().can_undo());
    h.get_by_label("Redo irradiance").click();
    h.run_steps(3);
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &after);
    h.get_by_label("Save irradiance").click();
    h.run_steps(3);
    assert!(!h.state().0.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        IrradianceBindings::open(sidecar).unwrap().document(),
        &after
    );
    assert_eq!(fs::read(path).unwrap(), scene);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(h.state().1.checksum(), checksum);
}
#[test]
fn invalid_gains_and_coefficient_range_overflow_do_not_mutate_document_or_history() {
    let (_temp, path, editor, mut panel) = fixture();
    let before = panel.bindings.as_ref().unwrap().document().clone();
    let revision = panel.bindings.as_ref().unwrap().revision();
    let sidecar = panel.bindings.as_ref().unwrap().path.clone();
    let saved = fs::read(&sidecar).unwrap();
    let scene = fs::read(&path).unwrap();
    let history = editor.history().clone();
    for channel in 0..3 {
        for value in [-0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 16.001] {
            panel.coefficient_gain = [1.0; 3];
            panel.coefficient_gain[channel] = value;
            assert!(panel.apply_coefficient_gain(&editor).is_err());
            let b = panel.bindings.as_ref().unwrap();
            assert_eq!(b.document(), &before);
            assert_eq!(b.revision(), revision);
            assert!(!b.can_undo());
            assert!(!b.dirty());
        }
    }
    let b = panel.bindings.as_mut().unwrap();
    let mut grid = b.document().grid.clone();
    grid.coefficients.last_mut().unwrap()[8][2] = 10_000.0;
    b.replace_grid(grid).unwrap();
    let overflow_before = b.document().clone();
    let revision = b.revision();
    panel.coefficient_gain = [1.0, 1.0, 2.0];
    assert!(panel.apply_coefficient_gain(&editor).is_err());
    assert_eq!(
        panel.bindings.as_ref().unwrap().document(),
        &overflow_before
    );
    assert_eq!(panel.bindings.as_ref().unwrap().revision(), revision);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
    assert_eq!(fs::read(path).unwrap(), scene);
    assert_eq!(editor.history(), &history);
}
#[test]
fn identity_preserves_baked_receipt_dirty_state_revision_and_existing_redo() {
    let (_temp, _path, editor, mut panel) = fixture();
    let clean = panel.bindings.as_ref().unwrap().document().clone();
    let clean_revision = panel.bindings.as_ref().unwrap().revision();
    panel.apply_coefficient_gain(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &clean);
    assert_eq!(panel.bindings.as_ref().unwrap().revision(), clean_revision);
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert!(!panel.bindings.as_ref().unwrap().can_undo());
    let b = panel.bindings.as_mut().unwrap();
    let mut grid = b.document().grid.clone();
    grid.provenance = IrradianceProvenance::Baked;
    b.commit_baked(b.revision(), grid, receipt()).unwrap();
    let baked = b.document().clone();
    panel.coefficient_gain = [2.0; 3];
    panel.apply_coefficient_gain(&editor).unwrap();
    let authored = panel.bindings.as_ref().unwrap().document().clone();
    panel.undo(&editor).unwrap();
    let b = panel.bindings.as_ref().unwrap();
    let revision = b.revision();
    let dirty = b.dirty();
    assert!(b.can_redo());
    panel.coefficient_gain = [1.0; 3];
    panel.apply_coefficient_gain(&editor).unwrap();
    let b = panel.bindings.as_ref().unwrap();
    assert_eq!(b.document(), &baked);
    assert_eq!(b.revision(), revision);
    assert_eq!(b.dirty(), dirty);
    assert!(b.can_redo());
    panel.redo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &authored);
}
#[test]
fn changed_baked_gain_clears_receipt_and_rejects_old_revision_commit_without_extra_undo() {
    let (_temp, _path, editor, mut panel) = fixture();
    let b = panel.bindings.as_mut().unwrap();
    let mut grid = b.document().grid.clone();
    grid.provenance = IrradianceProvenance::Baked;
    b.commit_baked(b.revision(), grid, receipt()).unwrap();
    let baked = b.document().clone();
    let old_revision = b.revision();
    panel.coefficient_gain = [0.5, 2.0, 1.0];
    panel.apply_coefficient_gain(&editor).unwrap();
    let b = panel.bindings.as_mut().unwrap();
    let authored = b.document().clone();
    let revision = b.revision();
    assert_eq!(authored.grid.provenance, IrradianceProvenance::Authored);
    assert!(authored.bake_receipt.is_none());
    assert_eq!(revision, old_revision + 1);
    // Exact stale publication boundary, not a claim about a spawned tracing job.
    assert!(b
        .commit_baked(
            old_revision,
            baked.grid.clone(),
            baked.bake_receipt.clone().unwrap()
        )
        .is_err());
    assert_eq!(b.document(), &authored);
    assert_eq!(b.revision(), revision);
    panel.undo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &baked);
    panel.redo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &authored);
}
#[test]
fn play_preview_and_changed_scene_reject_gain_without_writes() {
    for guard in 0..3 {
        let (temp, path, mut editor, mut panel) = fixture();
        panel.coefficient_gain = [2.0; 3];
        let before = panel.bindings.as_ref().unwrap().document().clone();
        let revision = panel.bindings.as_ref().unwrap().revision();
        let sidecar = panel.bindings.as_ref().unwrap().path.clone();
        let saved = fs::read(&sidecar).unwrap();
        match guard {
            0 => {
                assert!(editor.start_play());
                settle(&mut editor);
            }
            1 => {
                let agent = editor.agent_client("irradiance-gain-preview-test").unwrap();
                let id = agent
                    .call("proposal.begin", serde_json::json!({"label":"gain guard"}))
                    .unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                editor.sync();
                assert!(editor.set_preview(Some(id)));
                editor.sync();
                assert!(editor.previewing().is_some());
            }
            _ => {
                let other = temp.path().join("other.yaml");
                fs::copy(&path, &other).unwrap();
                assert!(editor.open_path(&other));
                settle(&mut editor);
            }
        }
        panel.sync_for_editor(&editor);
        assert!(panel.apply_coefficient_gain(&editor).is_err());
        let b = panel.bindings.as_ref().unwrap();
        assert_eq!(b.document(), &before);
        assert_eq!(b.revision(), revision);
        assert!(!b.dirty());
        assert_eq!(fs::read(sidecar).unwrap(), saved);
    }
}
