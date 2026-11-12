//! CPU UI -> saved schema -> shared native lighting consumer; no GPU claim.
use super::*;
use egui::accesskit::{Action, ActionData, ActionRequest};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{collections::BTreeMap, fs};
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(at).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().into(), fs::read(p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn open(root: &Path) -> orr_sample::room_project::PreparedProject {
    orr_sample::room_project::PreparedProject::open_with_lighting(root, true).unwrap()
}
fn saved_environment() -> (tempfile::TempDir, PathBuf, Document) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("environment-room");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "room-escape-3d-v1".into(),
        seed: "environment-acceptance".into(),
    })
    .unwrap();
    let manifest_path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["lighting"] = serde_json::json!("room.lighting.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let initial = Document::enabled_default();
    fs::write(root.join("room.lighting.json"), initial.to_bytes().unwrap()).unwrap();
    let mut prepared = open(&root);
    let frame = prepared.scene().frame().to_bytes();
    let point = prepared.point_light_settings();
    let lighting = prepared.take_lighting().unwrap();
    let (_, path, scene, _models) = prepared.into_parts();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor
        .install_room_lighting(lighting.document.clone())
        .unwrap();
    editor.sync();
    let panel = Panel::new_for_editor(lighting, &editor).unwrap();
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let baseline = files(&root);
    let mut h = Harness::builder()
        .with_size([1400.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut (Panel, crate::Editor)| {
                state.0.show_for_editor(ui, &mut state.1);
            },
            (panel, editor),
        );
    h.run_steps(3);
    h.get_by_label("Room lighting").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Environment lighting enabled")
        .click_accesskit();
    h.run_steps(3);
    h.get_by_label("Environment lighting controls")
        .click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        ("Sun direction X", -0.25),
        ("Sun direction Y", -0.75),
        ("Sun direction Z", 0.5),
        ("Sun linear R", 0.2),
        ("Sun linear G", 0.4),
        ("Sun linear B", 0.7),
        ("Sun intensity", 0.8),
        ("Sky linear R", 0.1),
        ("Sky linear G", 0.2),
        ("Sky linear B", 0.3),
        ("Ground linear R", 0.04),
        ("Ground linear G", 0.08),
        ("Ground linear B", 0.12),
        ("Environment ambient", 0.3),
    ] {
        let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
        h.event(egui::Event::AccessKitActionRequest(ActionRequest {
            target_node,
            target_tree,
            action: Action::SetValue,
            data: Some(ActionData::NumericValue(value)),
        }));
        h.run_steps(3);
    }
    let expected = h.state().0.candidate.clone();
    let environment = expected.environment.as_ref().unwrap();
    assert_eq!(environment.direction, [-0.25, -0.75, 0.5]);
    assert_eq!(environment.color, [0.2, 0.4, 0.7]);
    assert_eq!(environment.intensity, 0.8);
    assert_eq!(environment.sky, [0.1, 0.2, 0.3]);
    assert_eq!(environment.ground, [0.04, 0.08, 0.12]);
    assert_eq!(environment.ambient, 0.3);
    assert_eq!(h.state().0.document(), &initial);
    assert_eq!(h.state().1.room_lighting_document(), Some(&initial));
    assert!(h.state().0.undo.is_empty() && h.state().0.redo.is_empty());
    assert_eq!(files(&root), baseline);
    for (label, document) in [
        ("Apply lighting", expected.clone()),
        ("Undo lighting edit", initial.clone()),
        ("Redo lighting edit", expected.clone()),
    ] {
        h.get_by_label(label).click_accesskit();
        h.run_steps(3);
        assert!(
            h.state().0.error.is_none(),
            "{label}: {:?}",
            h.state().0.error
        );
        assert_eq!(h.state().0.document(), &document);
        assert_eq!(h.state().1.room_lighting_document(), Some(&document));
        let mut list = orr_render::RenderList3D::default();
        list.lighting = orr_sample::room_view::presentation_lighting(None).unwrap();
        h.state()
            .1
            .apply_room_environment_for_presentation(&mut list.lighting)
            .unwrap();
        assert_eq!(
            list.lighting,
            orr_sample::room_view::presentation_lighting(document.environment.as_ref()).unwrap()
        );
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(files(&root), baseline);
    }
    h.get_by_label("Save lighting").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let reopened = open(&root);
    assert_eq!(reopened.lighting().unwrap().document, expected);
    assert_eq!(reopened.scene().frame().to_bytes(), frame);
    assert_eq!(reopened.point_light_settings(), point);
    let actual = orr_sample::room_view::presentation_lighting(
        reopened.lighting().unwrap().document.environment.as_ref(),
    )
    .unwrap();
    let mut legacy = orr_sample::yard3d_view::yard_lighting();
    legacy.shadows = false;
    let mut expected_light = legacy;
    expected
        .environment
        .as_ref()
        .unwrap()
        .apply_to(&mut expected_light)
        .unwrap();
    assert_eq!(actual, expected_light);
    assert_eq!(actual.shadows, legacy.shadows);
    assert_eq!(
        (
            actual.shadow_radius,
            actual.shadow_center,
            actual.tonemap,
            actual.exposure
        ),
        (
            legacy.shadow_radius,
            legacy.shadow_center,
            legacy.tonemap,
            legacy.exposure
        )
    );
    assert_ne!(actual, legacy);
    let mut after = files(&root);
    after.remove(Path::new("room.lighting.json"));
    let mut before = baseline;
    before.remove(Path::new("room.lighting.json"));
    assert_eq!(after, before);
    (temp, root, expected)
}
#[test]
fn actual_environment_widgets_stage_apply_history_save_reopen_and_feed_shared_lighting() {
    let _ = saved_environment();
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "requires explicit trusted compiled Room runtime; CPU/headless only"]
fn saved_environment_survives_source_hidden_native_export_without_frame_changes() {
    use sha2::{Digest, Sha256};
    let (temp, root, expected) = saved_environment();
    let prepared = open(&root);
    let mut sim = prepared.scene().simulation().unwrap();
    for _ in 0..8 {
        let mut inputs = orr_sim::TickInputs::new(sim.tick() + 1, 1);
        inputs.set_input(
            orr_sim::PlayerSlot(0),
            orr_sample::room_game::RoomInput {
                move_x: 1,
                buttons: orr_sample::room_game::INTERACT,
                ..Default::default()
            },
        );
        sim.step(&inputs);
    }
    let expected_frame = format!("room tick: 8 checksum: 0x{:016x}", sim.frame().checksum());
    let runtime = temp.path().join("trusted-runtime");
    fs::copy(
        std::env::var_os("ORR_ROOM_ENVIRONMENT_RUNTIME").expect("explicit built runtime"),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = temp.path().join("environment-game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    fs::rename(&root, root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let deployed = open(&bundle.join("project"));
    assert_eq!(deployed.lighting().unwrap().document, expected);
    let lighting = orr_sample::room_view::presentation_lighting(
        deployed.lighting().unwrap().document.environment.as_ref(),
    )
    .unwrap();
    assert_eq!(lighting.intensity, 0.8);
    assert_eq!(lighting.ambient, 0.3);
    let output = std::process::Command::new(bundle.join("run-room-escape"))
        .current_dir(temp.path())
        .args([
            "--headless",
            "--ticks",
            "8",
            "--hold",
            "right,interact",
            "--check-model-poses",
        ])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(&expected_frame), "{text}");
    assert!(
        text.contains("room model poses: 9 frames checked;"),
        "{text}"
    );
}
