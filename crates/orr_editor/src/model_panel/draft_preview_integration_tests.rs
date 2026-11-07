//! CPU-only actual authoring controls -> ephemeral placement -> Cancel/Assign.
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
fn fixture(animated: bool) -> (tempfile::TempDir, Harness<'static, (ModelPanel, Editor)>) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let model = oracle::fixture();
    fs::write(
        source.join("model.animated.json"),
        model.to_bytes().unwrap(),
    )
    .unwrap();
    let static_model = orr_model::import::import_with_resolver(
        "fixture.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    fs::write(
        source.join("model.model.json"),
        static_model.to_bytes().unwrap(),
    )
    .unwrap();
    fs::write(source.join("LICENSE.txt"), b"Acceptance fixture license").unwrap();
    fs::write(source.join("orr.package.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"name":"pose-fit","version":"1.0.0","engine":"^0.0.1","capabilities":["animation"],"dependencies":{},"files":["model.animated.json","model.model.json","LICENSE.txt"]})).unwrap()).unwrap();
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
    let text = include_str!("../../../../scenes/yard3d_authoring.scene.yaml")
        .replace("kind: dynamic", "kind: static")
        .replace("inv_mass: 1", "inv_mass: 0");
    fs::write(&scene, text).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    let mut panel = ModelPanel::default();
    panel.open_for_editor(&editor, true).unwrap();
    panel.project = ".".into();
    panel.package = "pose-fit".into();
    panel.asset = if animated {
        "model.animated.json"
    } else {
        "model.model.json"
    }
    .into();
    panel.kind = if animated {
        ModelKind::Animated
    } else {
        ModelKind::Static
    };
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
    assert_eq!(
        h.state().0.asset,
        if animated {
            "model.animated.json"
        } else {
            "model.model.json"
        }
    );
    assert_eq!(
        h.state().0.kind,
        if animated {
            ModelKind::Animated
        } else {
            ModelKind::Static
        }
    );
    let selected = h.state().1.selected_guid().unwrap().to_string();
    let binding = &h.state().0.bindings.as_ref().unwrap().document().bindings[&selected];
    assert_eq!(binding.package, "pose-fit");
    assert_eq!(binding.kind, h.state().0.kind);
    let (package, asset, kind, loaded) = h
        .state()
        .0
        .candidate
        .as_ref()
        .expect("actual Load model asset must retain the selected animated candidate");
    assert_eq!(
        (package.as_str(), asset.as_str(), *kind),
        ("pose-fit", h.state().0.asset.as_str(), h.state().0.kind)
    );
    assert_eq!(loaded.source_hash(), binding.source_hash);
    assert_eq!(loaded.kind(), h.state().0.kind);
    (temp, h)
}

fn click(h: &mut Harness<'_, (ModelPanel, Editor)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
}
fn placement(h: &mut Harness<'_, (ModelPanel, Editor)>) -> Option<DraftPlacement> {
    let state = h.state_mut();
    state.0.draft_placement(&state.1)
}
#[test]
fn actual_static_and_animated_preview_cancel_then_assign_preserves_host_and_unrelated_bindings() {
    for animated in [false, true] {
        let (temp, mut h) = fixture(animated);
        let original = h.state().0.bindings.as_ref().unwrap().document().clone();
        let guid = h.state().1.selected_guid().unwrap().to_string();
        let entity = h
            .state()
            .1
            .rows()
            .iter()
            .find(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == guid))
            .unwrap()
            .entity;
        let before = files(temp.path());
        let frame = host_bytes(&mut h.state_mut().1);
        let history = h.state().1.history().clone();
        click(&mut h, "Fit rest model to box");
        let transform = h.state().0.transform;
        assert_ne!(transform, original.bindings[&guid].transform);
        if animated {
            h.get_by_label("Fit pose seconds").focus();
            h.run_steps(2);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.run_steps(2);
            h.get_by_label("Fit pose seconds").type_text("0.75");
            h.run_steps(3);
        }
        click(&mut h, "Preview model draft on selected object");
        let draft = placement(&mut h).expect("explicit Preview must provide placement");
        assert_eq!(draft.entity(), entity);
        match draft {
            DraftPlacement::Static(p) => {
                assert!(!animated);
                assert_eq!(p.entity, entity);
                let body = h
                    .state()
                    .1
                    .yard_frame()
                    .poses
                    .iter()
                    .find(|(e, _)| *e == entity)
                    .unwrap()
                    .1;
                assert_eq!(p.instance, ModelPanel::instance(body, transform));
            }
            DraftPlacement::Animated(p) => {
                assert!(animated);
                let expected = p.model.sample_clip(0, 0.75).unwrap();
                assert_eq!(p.pose, expected);
                let body = h
                    .state()
                    .1
                    .yard_frame()
                    .poses
                    .iter()
                    .find(|(e, _)| *e == entity)
                    .unwrap()
                    .1;
                assert_eq!(p.instance, ModelPanel::animated_instance(body, transform));
            }
        }
        assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
        assert_eq!(files(temp.path()), before);
        assert_eq!(host_bytes(&mut h.state_mut().1), frame);
        assert_eq!(h.state().1.history(), &history);
        let normal_static = h.state().0.placements(&h.state().1);
        let normal_animated = h.state().0.animated_placements(&h.state().1).unwrap();
        assert_eq!(normal_static.len() + normal_animated.len(), 2);
        click(&mut h, "Cancel model draft preview");
        assert!(placement(&mut h).is_none());
        assert_eq!(h.state().0.transform, transform);
        assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
        click(&mut h, "Preview model draft on selected object");
        assert!(placement(&mut h).is_some());
        click(
            &mut h,
            if animated {
                "Assign animated model"
            } else {
                "Assign static model"
            },
        );
        assert!(placement(&mut h).is_none(), "Assign retires draft context");
        let assigned = h.state().0.bindings.as_ref().unwrap().document().clone();
        assert_eq!(assigned.bindings[&guid].transform, transform);
        for (key, binding) in &original.bindings {
            if key != &guid {
                assert_eq!(&assigned.bindings[key], binding);
            }
        }
        assert_eq!(files(temp.path()), before, "Assign is not Save");
        click(&mut h, "Save model bindings");
        let mut reopened = ModelPanel::default();
        reopened.open_for_editor(&h.state().1, false).unwrap();
        assert_eq!(reopened.bindings.as_ref().unwrap().document(), &assigned);
        assert_eq!(host_bytes(&mut h.state_mut().1), frame);
        assert_eq!(h.state().1.history(), &history);
    }
}
#[test]
fn changed_draft_or_selection_retires_preview_without_panel_render_or_binding_edit() {
    let (temp, mut h) = fixture(true);
    let original = h.state().0.bindings.as_ref().unwrap().document().clone();
    let before = files(temp.path());
    let checksum = h.state().1.checksum();
    click(&mut h, "Preview model draft on selected object");
    assert!(placement(&mut h).is_some());
    let transform = h.state().0.transform;
    h.state_mut().0.transform.translation[0] += 0.25;
    assert!(placement(&mut h).is_none());
    h.state_mut().0.transform = transform;
    assert!(
        placement(&mut h).is_none(),
        "restoring fields never revives retired preview"
    );
    click(&mut h, "Preview model draft on selected object");
    assert!(placement(&mut h).is_some());
    assert!(h.state_mut().1.select_named("box_left"));
    h.state_mut().1.sync();
    assert!(placement(&mut h).is_none());
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(files(temp.path()), before);
}
