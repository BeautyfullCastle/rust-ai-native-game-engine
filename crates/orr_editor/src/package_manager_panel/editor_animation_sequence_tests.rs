//! Actual CPU import widgets -> new package -> verified binding -> saved Room policy.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{
    fs,
    time::{Duration, Instant},
};
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn click<T>(h: &mut Harness<'_, T>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text<T>(h: &mut Harness<'_, T>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn open(root: &Path) -> orr_sample::room_project::PreparedProject {
    orr_sample::room_project::PreparedProject::open_with_capabilities(
        root,
        orr_sample::room_project::RoomProjectCapabilities {
            character: true,
            lighting: true,
        },
    )
    .unwrap()
}
fn wait(panel: &mut Panel) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while panel.busy() && Instant::now() < deadline {
        panel.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.busy(), "{}", panel.status);
}
pub(super) fn authored_sequence_room() -> crate::package_manager::presentation_tests::Fixture {
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let initial = open(&f.root);
    let before = initial.scene().frame().to_bytes();
    let before_checksum = initial.scene().frame().checksum();
    drop(initial);
    let source = f.temp.path().join("sequence-source.animated.json");
    fs::write(&source, SOURCE).unwrap();
    let license = f.temp.path().join("sequence-LICENSE.txt");
    fs::write(&license, b"Fixture rights explicitly reviewed").unwrap();
    let destination = f.temp.path().join("sequenced model package");
    let mut h = Harness::builder()
        .with_size([1600.0, 2600.0])
        .build_ui_state(
            |ui,
             s: &mut (
                crate::model_package_import_panel::Panel,
                PathBuf,
                Option<crate::model_package_import::Report>,
            )| {
                if let Some(report) = s.0.poll(Some(&s.1), 4, 7) {
                    s.2 = Some(report);
                }
                s.0.show(ui, true, Some(&s.1), 4, 7);
            },
            (
                crate::model_package_import_panel::Panel::default(),
                f.root.clone(),
                None,
            ),
        );
    h.run_steps(3);
    click(&mut h, "Import a model package");
    click(&mut h, "Cooked animated model");
    for (label, value) in [
        (
            "Cooked animated model file (.animated.json)",
            source.to_str().unwrap(),
        ),
        ("License file", license.to_str().unwrap()),
        (
            "Package destination directory",
            destination.to_str().unwrap(),
        ),
        ("Package name", "sequenced-courier"),
        ("Package version", "1.0.0"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.query_by_label("Clip 2: escaped (1.000s)").is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    click(&mut h, "Create animation clip sequence");
    click(&mut h, "Sequence first clip");
    click(&mut h, "Sequence first clip 0: searching (2.000s)");
    click(&mut h, "Sequence second clip");
    click(&mut h, "Sequence second clip 2: escaped (1.000s)");
    text(&mut h, "Sequence transition seconds", "0.25");
    text(&mut h, "New sequence clip name", "sequenced-search");
    text(
        &mut h,
        "Sequence variant asset ID",
        "variants/courier.sequence.json",
    );
    assert!(h
        .get_by_label("Import cooked animated package")
        .accesskit_node()
        .is_disabled());
    let preview_files = snapshot(f.temp.path());
    click(&mut h, "Preview animated variant");
    let deadline = Instant::now() + Duration::from_secs(10);
    let preview = loop {
        h.run_steps(3);
        if let Some(preview) = h.state_mut().0.preview_frame() {
            break preview;
        }
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    };
    let original = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    assert_eq!(preview.prepared.model.source().clips.len(), 4);
    assert_eq!(
        &preview.prepared.model.source().clips[..3],
        original.source().clips.as_slice()
    );
    assert_eq!(
        preview.prepared.model.source().clips[3].name,
        "sequenced-search"
    );
    assert_eq!(preview.prepared.model.source().clips[3].duration(), 3.25);
    assert!(!destination.exists());
    assert_eq!(
        snapshot(f.temp.path()),
        preview_files,
        "preview cannot publish or rewrite inputs/project"
    );
    assert!(h.state().2.is_none());
    assert_eq!(open(&f.root).scene().frame().to_bytes(), before);
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    assert_eq!(
        fs::read(&license).unwrap(),
        b"Fixture rights explicitly reviewed"
    );
    click(&mut h, "I have reviewed rights to distribute this content");
    click(&mut h, "Import cooked animated package");
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().2.is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = h.state_mut().2.take().unwrap();
    assert_eq!(report.source_sha256, preview.prepared.asset_sha256);
    assert_eq!(
        fs::read(report.root.join(&report.asset)).unwrap(),
        preview.prepared.model.to_bytes().unwrap()
    );
    assert_eq!(report.package, "sequenced-courier");
    drop(h);
    let prepared = open(&f.root);
    let character = prepared.character().unwrap().document.clone();
    let guid = orr_reflect::Guid::parse(&character.player).unwrap();
    let old = prepared.models().document.bindings[&character.player].clone();
    drop(prepared);
    let mut packages = Panel {
        root: Some(f.root.clone()),
        source: report.root.display().to_string(),
        source_identity: report.root.display().to_string(),
        ..Default::default()
    };
    packages.start_preview(report.root.clone());
    wait(&mut packages);
    assert!(packages.preview.is_some(), "{}", packages.status);
    packages.start_add();
    wait(&mut packages);
    packages.start_verify();
    wait(&mut packages);
    assert!(packages
        .verified
        .as_ref()
        .unwrap()
        .direct
        .contains_key(&report.package));
    let asset = crate::model_bindings::load_asset_for_kind(
        &f.root,
        &report.package,
        &report.asset,
        ModelKind::Animated,
    )
    .unwrap();
    let binding = crate::model_bindings::Binding::from_animated_asset(
        report.package.clone(),
        report.asset.clone(),
        &asset,
        old.animation.unwrap(),
        old.transform,
    )
    .unwrap();
    let mut bindings =
        crate::model_bindings::Bindings::open(f.root.join("room.models.json")).unwrap();
    bindings
        .assign_validated(std::slice::from_ref(&guid), &binding, &asset)
        .unwrap();
    bindings.save().unwrap();
    let mut prepared = open(&f.root);
    let character = prepared.take_character().unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor
        .install_room_character(character.document.clone())
        .unwrap();
    let mut model_panel = crate::model_panel::ModelPanel::default();
    model_panel.install_room(models).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.sync();
        if editor.yard_rows_coherent() && model_panel.scene_matches(&editor) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let panel = crate::room_character_panel::Panel::new_for_editor(character, &editor).unwrap();
    let mut h = Harness::builder()
        .with_size([1600.0, 2600.0])
        .build_ui_state(
            |ui,
             s: &mut (
                crate::Editor,
                crate::model_panel::ModelPanel,
                crate::room_character_panel::Panel,
            )| {
                s.0.sync();
                s.2.show(ui, &mut s.0, &mut s.1);
            },
            (editor, model_panel, panel),
        );
    h.run_steps(3);
    click(&mut h, "Room character");
    click(&mut h, "Searching clip");
    click(&mut h, "Searching clip 3: sequenced-search (3.25s)");
    // Enable the existing pickup policy as well. Searching idle inherits clip3;
    // neutral native playback therefore uses it and retains the existing pose diagnostic.
    click(&mut h, "Play key pickup clip once");
    click(&mut h, "Apply character settings");
    assert_eq!(h.state().2.document().searching, 3);
    assert_eq!(h.state().2.document().schema, 6);
    let authored = h.state().2.document().clone();
    assert_eq!(authored.motion.unwrap().searching_idle, 3);
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().2.document().searching, 0);
    click(&mut h, "Redo character edit");
    assert_eq!(h.state().2.document(), &authored);
    click(&mut h, "Save character and model bindings");
    assert!(!h.state().2.dirty());
    assert_eq!(
        h.state().0.snapshot().unwrap().predicted().checksum(),
        before_checksum
    );
    drop(h);
    let reopened = open(&f.root);
    assert_eq!(reopened.character().unwrap().document, authored);
    let saved = &reopened.models().document.bindings[&authored.player];
    assert_eq!(saved.package, "sequenced-courier");
    assert_eq!(saved.animation.unwrap().clip_index, 3);
    assert_eq!(saved.transform, old.transform);
    assert_eq!(reopened.scene().frame().to_bytes(), before);
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    assert_eq!(
        fs::read(&license).unwrap(),
        b"Fixture rights explicitly reviewed"
    );
    f
}
#[test]
fn actual_sequence_widgets_preview_import_install_appended_clip_room_save_reopen_and_sample_boundaries(
) {
    let fixture = authored_sequence_room();
    assert_saved_sequence(&fixture.root);
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00003, "{a} != {b}");
}
fn rotations_close(a: [f32; 4], b: [f32; 4]) {
    let dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    for i in 0..4 {
        close(a[i], if dot < 0. { -b[i] } else { b[i] });
    }
}
// Independent shortest-arc quaternion interpolation, not the sequence producer.
fn slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    if dot < 0. {
        dot = -dot;
        b = b.map(|v| -v);
    }
    let mut q = [0.; 4];
    if dot > 0.9995 {
        for i in 0..4 {
            q[i] = a[i] + t * (b[i] - a[i]);
        }
    } else {
        let angle = dot.clamp(-1., 1.).acos();
        let denom = angle.sin();
        for i in 0..4 {
            q[i] = ((1. - t) * angle).sin() / denom * a[i] + (t * angle).sin() / denom * b[i];
        }
    }
    let norm = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    q.map(|v| v / norm)
}
fn expected_local(
    old: &orr_model::animation::AnimatedModel,
    time: f32,
) -> Vec<orr_model::animation::Trs> {
    if time <= 2. {
        return old.sample_clip(0, time).unwrap().local().to_vec();
    }
    if time >= 2.25 {
        return old.sample_clip(2, time - 2.25).unwrap().local().to_vec();
    }
    let a = old.sample_clip(0, 2.).unwrap();
    let b = old.sample_clip(2, 0.).unwrap();
    let t = (time - 2.) / 0.25;
    a.local()
        .iter()
        .zip(b.local())
        .map(|(a, b)| {
            let mut out = *a;
            for axis in 0..3 {
                out.translation[axis] =
                    a.translation[axis] + t * (b.translation[axis] - a.translation[axis]);
                out.scale[axis] = a.scale[axis] + t * (b.scale[axis] - a.scale[axis]);
            }
            out.rotation = slerp(a.rotation, b.rotation, t);
            out
        })
        .collect()
}
fn assert_local(pose: &orr_model::animation::Pose, expected: &[orr_model::animation::Trs]) {
    assert_eq!(pose.local().len(), expected.len());
    for (a, b) in pose.local().iter().zip(expected) {
        for axis in 0..3 {
            close(a.translation[axis], b.translation[axis]);
            close(a.scale[axis], b.scale[axis]);
        }
        rotations_close(a.rotation, b.rotation);
    }
}
fn assert_saved_sequence(root: &Path) {
    let prepared = open(root);
    let character = &prepared.character().unwrap().document;
    let binding = &prepared.models().document.bindings[&character.player];
    assert_eq!(binding.animation.unwrap().clip_index, 3);
    let model = prepared.models().assets[&(binding.package.clone(), binding.asset.clone())]
        .animated_model()
        .unwrap();
    let old = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    assert_eq!(model.source().clips.len(), old.source().clips.len() + 1);
    assert_eq!(model.source().clips[3].duration(), 3.25);
    let mut restored = model.source().clone();
    restored.asset_id = old.source().asset_id.clone();
    restored.dependencies = old.source().dependencies.clone();
    restored.clips.pop();
    for (a, b) in old.source().primitives.iter().zip(&mut restored.primitives) {
        b.id = a.id.clone();
    }
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(old.source()).unwrap(),
        "original clips/indices, channels, geometry, materials and rig preserved"
    );
    for clip in 0..3 {
        for time in [0., 0.25, 0.75] {
            let a = old.sample_clip(clip, time).unwrap();
            let b = model.sample_clip(clip, time).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
        }
    }
    // Source-derived endpoint and midpoint oracle includes the exact full B end.
    for time in [0., 1., 2., 2.125, 2.25, 2.75, 3.25] {
        assert_local(
            &model.sample_clip(3, time).unwrap(),
            &expected_local(&old, time),
        );
    }
    let mut sim = prepared.scene().simulation().unwrap();
    let mut playback = orr_sample::room_character::PlaybackCursor::default();
    let rate = orr_sample::room_game::TICK_RATE;
    for tick in 0..=rate as u64 * 3 {
        if tick > 0 {
            sim.step(&orr_sim::TickInputs::new(sim.tick() + 1, 1));
        }
        let frame = orr_bridge::FrameView::of(sim.frame());
        let bytes = sim.frame().to_bytes();
        orr_sample::room_view::observe_character(
            &mut playback,
            frame,
            prepared.scene().index(),
            prepared.models(),
            orr_sample::room_view::PresentationTime::default(),
        )
        .unwrap();
        if [
            rate as u64,
            rate as u64 * 2,
            rate as u64 * 2 + 8,
            rate as u64 * 9 / 4,
            rate as u64 * 5 / 2,
            rate as u64 * 3,
        ]
        .contains(&tick)
        {
            let placed = orr_sample::room_view::observed_character_placement(
                frame,
                prepared.scene().index(),
                prepared.models(),
                &playback,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                placed.model.source().asset_id,
                "variants/courier.sequence.json"
            );
            assert_local(
                &placed.pose,
                &expected_local(&old, tick as f32 / rate as f32),
            );
            assert!(!placed.model.deform(&placed.pose).unwrap().is_empty());
            assert!(
                orr_sample::room_view::check_model_poses(
                    frame,
                    prepared.scene().index(),
                    prepared.models(),
                    Some(&playback)
                )
                .unwrap()
                .animated_models
                    > 0
            );
        }
        assert_eq!(sim.frame().to_bytes(), bytes, "CPU consumer is read-only");
    }
}
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.push((p.strip_prefix(root).unwrap().into(), Vec::new()));
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
