//! Linux CPU acceptance: sampled pose -> local draft -> binding transaction and consumer.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "../../../orr_model/tests/common/animation_clip_sequence_oracle.rs"]
mod oracle;
fn host_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("pose-fit-readonly-acceptance").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(Instant::now() < deadline, "immutable host frame timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out);
            } else {
                out.push((p.strip_prefix(root).unwrap().into(), fs::read(p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}
fn fixture(sphere: bool) -> (tempfile::TempDir, Harness<'static, (ModelPanel, Editor)>) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let model = oracle::fixture();
    fs::write(
        source.join("model.animated.json"),
        model.to_bytes().unwrap(),
    )
    .unwrap();
    fs::write(source.join("LICENSE.txt"), b"Acceptance fixture license").unwrap();
    fs::write(source.join("orr.package.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"name":"pose-fit","version":"1.0.0","engine":"^0.0.1","capabilities":["animation"],"dependencies":{},"files":["model.animated.json","LICENSE.txt"]})).unwrap()).unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    orr_package::Project::open_for_install(
        &root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source])
    .unwrap();
    let scene = root.join("yard.scene.yaml");
    let mut text = include_str!("../../../../scenes/yard3d_authoring.scene.yaml")
        .replace("kind: dynamic", "kind: static")
        .replace("inv_mass: 1", "inv_mass: 0");
    if sphere {
        text = text.replace(
            "shape: { kind: box, half_extents: [0.5, 0.5, 0.5] }",
            "shape: { kind: sphere, radius: 0.5 }",
        );
    }
    fs::write(&scene, text).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    let mut panel = ModelPanel::default();
    panel.open_for_editor(&editor, true).unwrap();
    panel.project = ".".into();
    panel.package = "pose-fit".into();
    panel.asset = "model.animated.json".into();
    panel.kind = ModelKind::Animated;
    panel.animation.clip_index = 0;
    assert!(editor.select_named("box_left"));
    editor.sync();
    panel.assign(&editor).unwrap();
    assert!(editor.select_named("box_right"));
    editor.sync();
    panel.transform.scale = [0.25; 3];
    panel.assign(&editor).unwrap();
    panel.save(&editor).unwrap();
    let mut h = Harness::builder()
        .with_size([1400.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut (ModelPanel, Editor)| state.0.show(ui, &state.1),
            (panel, editor),
        );
    h.run_steps(3);
    h.get_by_label("Static model binding").click_accesskit();
    h.run_steps(3);
    // First show synchronizes the scene identity and reopens its saved sidecar,
    // deliberately retiring any candidate loaded before that refresh.
    h.get_by_label("Use selected model settings")
        .click_accesskit();
    h.run_steps(3);
    h.get_by_label("Load model asset").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    assert_eq!(h.state().0.package, "pose-fit");
    assert_eq!(h.state().0.asset, "model.animated.json");
    assert_eq!(h.state().0.kind, ModelKind::Animated);
    let selected = h.state().1.selected_guid().unwrap().to_string();
    let binding = &h.state().0.bindings.as_ref().unwrap().document().bindings[&selected];
    assert_eq!(binding.package, "pose-fit");
    assert_eq!(binding.kind, ModelKind::Animated);
    let (package, asset, kind, loaded) = h
        .state()
        .0
        .candidate
        .as_ref()
        .expect("actual Load model asset must retain the selected animated candidate");
    assert_eq!(
        (package.as_str(), asset.as_str(), *kind),
        ("pose-fit", "model.animated.json", ModelKind::Animated)
    );
    assert_eq!(loaded.source_hash(), binding.source_hash);
    assert!(loaded.animated_model().is_some());
    (temp, h)
}
fn stage(h: &mut Harness<'_, (ModelPanel, Editor)>, sphere: bool, time: &str) {
    let loaded = h
        .state()
        .0
        .candidate
        .as_ref()
        .unwrap()
        .3
        .animated_model()
        .unwrap();
    let clip = &loaded.source().clips[1];
    let label = format!("1: {} ({:.2}s)", clip.name, clip.duration());
    if h.state().0.animation.clip_index != 1 {
        h.get_by_label("Animation clip").click_accesskit();
        h.run_steps(3);
        h.get_by_label(&label).click_accesskit();
        h.run_steps(3);
    }
    h.get_by_label("Fit pose seconds").focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_label("Fit pose seconds").type_text(time);
    h.run_steps(3);
    h.get_by_label(if sphere {
        "Fit sampled pose to sphere"
    } else {
        "Fit sampled pose to box"
    })
    .click_accesskit();
    h.run_steps(3);
}
#[test]
fn sampled_box_and_sphere_fit_is_draft_then_assign_undo_redo_save_reopen_and_contains_world_points()
{
    for sphere in [false, true] {
        let (temp, mut h) = fixture(sphere);
        let before = files(temp.path());
        let history = h.state().1.history().clone();
        let original = h.state().0.bindings.as_ref().unwrap().document().clone();
        let guid = h.state().1.selected_guid().unwrap().to_string();
        let frame = host_bytes(&mut h.state_mut().1);
        let model = oracle::fixture();
        let pose = model.sample_clip(1, 0.75).unwrap();
        assert_ne!(
            model.bounds(&pose).unwrap(),
            model.bounds(&model.rest_pose().unwrap()).unwrap(),
            "fixture sample must differ from rest bounds"
        );
        stage(&mut h, sphere, "0.75");
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        let fit = h.state().0.transform;
        assert_eq!(fit.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(fit.scale[0], fit.scale[1]);
        assert_eq!(fit.scale[1], fit.scale[2]);
        assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
        assert_eq!(files(temp.path()), before);
        assert_eq!(host_bytes(&mut h.state_mut().1), frame);
        let rest = if sphere {
            fit::loaded_sphere(&h.state().0.candidate.as_ref().unwrap().3, 0.5)
        } else {
            fit::loaded(&h.state().0.candidate.as_ref().unwrap().3, [0.5; 3])
        }
        .unwrap();
        assert_ne!(fit, rest, "sample fit must not silently reuse rest bounds");
        h.get_by_label("Assign animated model").click_accesskit();
        h.run_steps(3);
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        let assigned = h.state().0.bindings.as_ref().unwrap().document().clone();
        assert_eq!(assigned.bindings[&guid].transform, fit);
        assert_eq!(assigned.bindings[&guid].animation.unwrap().clip_index, 1);
        for (key, binding) in &original.bindings {
            if key != &guid {
                assert_eq!(&assigned.bindings[key], binding);
            }
        }
        h.get_by_label("Undo model").click_accesskit();
        h.run_steps(3);
        assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
        h.get_by_label("Redo model").click_accesskit();
        h.run_steps(3);
        assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &assigned);
        h.get_by_label("Save model bindings").click_accesskit();
        h.run_steps(3);
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        let mut reopened = ModelPanel::default();
        reopened.open_for_editor(&h.state().1, false).unwrap();
        assert_eq!(reopened.bindings.as_ref().unwrap().document(), &assigned);
        let row = h
            .state()
            .1
            .rows()
            .iter()
            .find(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == guid))
            .unwrap();
        let body = *h
            .state()
            .1
            .yard_frame()
            .poses
            .iter()
            .find(|(entity, _)| *entity == row.entity)
            .unwrap();
        let instance = ModelPanel::animated_instance(body.1, assigned.bindings[&guid].transform);
        let loaded = &reopened.loaded[&guid];
        let model = loaded.animated_model().unwrap();
        let deformed = model.deform(&model.sample_clip(1, 0.75).unwrap()).unwrap();
        for primitive in deformed {
            for vertex in primitive.vertices {
                let world: [f64; 3] = std::array::from_fn(|axis| {
                    (0..3)
                        .map(|column| {
                            f64::from(instance[column][axis]) * f64::from(vertex.position[column])
                        })
                        .sum::<f64>()
                        + f64::from(instance[3][axis])
                });
                let local = std::array::from_fn::<_, 3, _>(|axis| {
                    world[axis] - f64::from(body.1.pos.to_array()[axis])
                });
                if sphere {
                    assert!(local.iter().map(|v| v * v).sum::<f64>().sqrt() <= 0.495001);
                } else {
                    assert!(local.iter().all(|v| v.abs() <= 0.495001));
                }
            }
        }
        assert_eq!(host_bytes(&mut h.state_mut().1), frame);
        assert_eq!(h.state().1.history(), &history);
        let after = files(temp.path());
        for (path, bytes) in before {
            if path
                != h.state()
                    .0
                    .bindings
                    .as_ref()
                    .unwrap()
                    .path
                    .strip_prefix(temp.path())
                    .unwrap()
            {
                assert_eq!(after.iter().find(|(p, _)| p == &path).unwrap().1, bytes);
            }
        }
    }
}
#[test]
fn invalid_time_and_changed_installed_bytes_refuse_without_draft_or_binding_changes() {
    let (temp, mut h) = fixture(false);
    let transform = h.state().0.transform;
    let document = h.state().0.bindings.as_ref().unwrap().document().clone();
    let checksum = h.state().1.checksum();
    stage(&mut h, false, "NaN");
    assert!(h.state().0.error.is_some());
    assert_eq!(h.state().0.transform, transform);
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &document);
    let installed = files(&temp.path().join("project"))
        .into_iter()
        .find(|(p, _)| {
            p.file_name()
                .is_some_and(|name| name == "model.animated.json")
        })
        .unwrap()
        .0;
    let path = temp.path().join("project").join(installed);
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' ');
    fs::write(&path, &bytes).unwrap();
    stage(&mut h, false, "0.75");
    assert!(h.state().0.error.is_some());
    assert_eq!(h.state().0.transform, transform);
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &document);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
