//! Linux CPU acceptance of actual import form -> worker -> installed binding.
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
const ORIGINAL: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn source_bytes() -> Vec<u8> {
    let model = orr_model::animation::AnimatedModel::from_bytes(ORIGINAL).unwrap();
    let mut source = model.source().clone();
    source.materials.push(source.materials[0].clone());
    let selected = (source.materials.len() - 1) as u32;
    for primitive in &mut source.primitives {
        primitive.material = selected;
    }
    let mut shared = source.primitives[0].clone();
    shared.material = 0;
    shared.id = format!(
        "{}#node={}/mesh=777/primitive=0",
        source.asset_id, shared.node
    );
    source.primitives.push(shared);
    orr_model::animation::AnimatedModel::new(source)
        .unwrap()
        .to_bytes()
        .unwrap()
}
fn material_index() -> u32 {
    orr_model::animation::AnimatedModel::from_bytes(&source_bytes())
        .unwrap()
        .source()
        .materials
        .len() as u32
        - 1
}
fn wait(panel: &mut Panel) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while panel.busy() && Instant::now() < deadline {
        panel.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.busy(), "worker timeout: {}", panel.status);
}
fn imported_player(quarter: u32) -> (crate::package_manager::presentation_tests::Fixture, Vec<u8>) {
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let source = f.temp.path().join("author.model.animated.json");
    fs::write(&source, source_bytes()).unwrap();
    let license = f.temp.path().join("author-LICENSE.txt");
    let license_bytes = b"Acceptance fixture: redistribution acknowledged";
    fs::write(&license, license_bytes).unwrap();
    let destination = f.temp.path().join("new animated package");
    let mut h = Harness::builder()
        .with_size([1400.0, 1800.0])
        .build_ui_state(
            |ui,
             state: &mut (
                crate::model_package_import_panel::Panel,
                PathBuf,
                Option<crate::model_package_import::Report>,
            )| {
                if let Some(report) = state.0.poll(Some(&state.1), 4, 7) {
                    state.2 = Some(report);
                }
                state.0.show(ui, true, Some(&state.1), 4, 7);
            },
            (
                crate::model_package_import_panel::Panel::default(),
                f.root.clone(),
                None,
            ),
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Cooked animated model").click_accesskit();
    h.run_steps(3);
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
        ("Package name", "imported-courier"),
        ("Package version", "1.0.0"),
    ] {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(3);
    }
    configure_uv(&mut h, f.temp.path(), &destination, quarter);
    h.get_by_label("I have reviewed rights to distribute this content")
        .click_accesskit();
    h.run_steps(3);
    assert!(!destination.exists());
    h.get_by_label("Import cooked animated package")
        .click_accesskit();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().2.is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = h.state_mut().2.take().unwrap();
    let payload = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(fs::read(&source).unwrap(), source_bytes());
    assert_eq!(fs::read(&license).unwrap(), license_bytes);
    assert_eq!(report.asset, "model.animated.json");
    let prepared = open(&f.root);
    let character = prepared.character().unwrap().document.clone();
    let light = prepared.lighting().unwrap().document.clone();
    let player = prepared
        .scene()
        .frame()
        .entities()
        .find(|&e| {
            prepared
                .scene()
                .frame()
                .get::<orr_sample::room_game::RoomActor>(e)
                .is_some_and(|a| a.kind == orr_sample::room_game::PLAYER)
        })
        .unwrap();
    let guid = prepared.scene().index().guid(player).unwrap().clone();
    let old = prepared.models().document.bindings[&guid.to_string()].clone();
    assert_ne!(old.package, report.package);
    drop(prepared);
    let mut panel = Panel {
        root: Some(f.root.clone()),
        source: report.root.display().to_string(),
        source_identity: report.root.display().to_string(),
        ..Default::default()
    };
    assert!(!package_manager::inspect(&f.root)
        .unwrap()
        .packages
        .contains_key(&report.package));
    panel.start_preview(report.root.clone());
    wait(&mut panel);
    assert!(panel.preview.is_some(), "{}", panel.status);
    // Preview is read-only; adding is an explicit second production operation.
    assert!(!package_manager::inspect(&f.root)
        .unwrap()
        .packages
        .contains_key(&report.package));
    panel.start_add();
    wait(&mut panel);
    panel.start_verify();
    wait(&mut panel);
    let lock = panel.verified.as_ref().expect("installed verified lock");
    assert!(lock.direct.contains_key(&report.package));
    let mut ui = Harness::builder()
        .with_size(egui::vec2(1100.0, 1200.0))
        .build_ui_state(
            |ui, panel: &mut Panel| {
                model_asset_buttons(
                    ui,
                    ModelBrowser {
                        lock: panel.verified.as_ref().unwrap(),
                        filter: &mut panel.model_filter,
                        preview_request: &mut panel.preview_request,
                    },
                    true,
                    &mut panel.model_request,
                    panel.root.as_ref(),
                    &panel.source,
                    (panel.project_generation, panel.source_generation),
                );
            },
            panel,
        );
    ui.run_steps(3);
    ui.get_by_label("Preview animated model imported-courier/model.animated.json")
        .click();
    ui.run_steps(3);
    let preview = ui.state_mut().take_preview_request().unwrap();
    assert_eq!(preview.package_id, report.package);
    assert_eq!(preview.asset_path, report.asset);
    assert_eq!(preview.kind, ModelKind::Animated);
    ui.get_by_label("Use animated model imported-courier/model.animated.json")
        .click();
    ui.run_steps(3);
    let (package, asset, kind) = ui.state_mut().take_model_asset_request().unwrap();
    assert_eq!(
        (&package, &asset, kind),
        (&report.package, &report.asset, ModelKind::Animated)
    );
    // This calls the same validated editor transaction used by assignment, not
    // a synthetic JSON rewrite or a native-window/GPU playtest.
    let loaded =
        crate::model_bindings::load_asset_for_kind(&f.root, &package, &asset, kind).unwrap();
    let binding = crate::model_bindings::Binding::from_animated_asset(
        package.clone(),
        asset.clone(),
        &loaded,
        old.animation.unwrap(),
        old.transform,
    )
    .unwrap();
    assert_eq!(binding.source_hash, report.source_sha256);
    assert!(loaded.animated_model().unwrap().source().clips.len() >= 3);
    let mut bindings =
        crate::model_bindings::Bindings::open(f.root.join("room.models.json")).unwrap();
    bindings
        .assign_validated(std::slice::from_ref(&guid), &binding, &loaded)
        .unwrap();
    assert!(bindings.dirty());
    bindings.save().unwrap();
    assert!(!bindings.dirty());
    let reopened = open(&f.root);
    let saved = &reopened.models().document.bindings[&guid.to_string()];
    assert_eq!(saved, &binding);
    assert_eq!(saved.package, "imported-courier");
    assert_eq!(saved.asset, "model.animated.json");
    assert_eq!(reopened.character().unwrap().document, character);
    assert_eq!(reopened.lighting().unwrap().document, light);
    assert!(reopened.models().assets.contains_key(&(package, asset)));
    assert_eq!(fs::read(&source).unwrap(), source_bytes());
    assert_eq!(fs::read(&license).unwrap(), license_bytes);
    (f, payload)
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
type ImportState = (
    crate::model_package_import_panel::Panel,
    PathBuf,
    Option<crate::model_package_import::Report>,
);
fn click(h: &mut Harness<'_, ImportState>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
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
fn configure_uv(h: &mut Harness<'_, ImportState>, root: &Path, destination: &Path, quarter: u32) {
    click(h, "Create animated UV variant");
    let before = snapshot(root);
    click(h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().0.busy() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(snapshot(root), before, "inspection cannot publish");
    click(h, &format!("Use material {}", material_index()));
    assert!(h
        .query_by_label(&format!("UV material: {}", material_index()))
        .is_some());
    for (label, value) in [
        ("UV offset U", "0.25"),
        ("UV offset V", "-0.5"),
        ("UV scale U", "2"),
        ("UV scale V", "0.5"),
    ] {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(3);
    }
    click(h, "UV quarter-turn rotation");
    click(
        h,
        if quarter == 1 {
            "90 degrees"
        } else {
            "270 degrees"
        },
    );
    h.get_by_role_and_label(Role::TextInput, "UV variant asset ID")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "UV variant asset ID")
        .type_text("variants/courier.uv.json");
    h.run_steps(3);
    assert!(
        h.get_by_label("Import cooked animated package")
            .accesskit_node()
            .is_disabled(),
        "selection cannot grant distribution consent"
    );
    assert_eq!(
        h.get_by_label("I have reviewed rights to distribute this content")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    let before = snapshot(root);
    click(h, "Preview animated variant");
    let deadline = Instant::now() + Duration::from_secs(10);
    let preview = loop {
        h.run_steps(3);
        if let Some(frame) = h.state_mut().0.preview_frame() {
            break frame;
        }
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_payload(&preview.prepared.model, quarter);
    assert!(h.state().2.is_none());
    assert!(!destination.exists());
    assert_eq!(
        snapshot(root),
        before,
        "preview cannot write package, project or inputs"
    );
    assert!(h
        .get_by_label("Import cooked animated package")
        .accesskit_node()
        .is_disabled());
}
#[test]
fn actual_uv_widgets_preview_import_install_assign_save_reopen_and_deformed_consumer() {
    for quarter in [1, 3] {
        let (f, bytes) = imported_player(quarter);
        let prepared = open(&f.root);
        let player = prepared
            .scene()
            .frame()
            .entities()
            .find(|&e| {
                prepared
                    .scene()
                    .frame()
                    .get::<orr_sample::room_game::RoomActor>(e)
                    .is_some_and(|a| a.kind == orr_sample::room_game::PLAYER)
            })
            .unwrap();
        let guid = prepared.scene().index().guid(player).unwrap();
        let binding = &prepared.models().document.bindings[&guid.to_string()];
        let loaded = &prepared.models().assets[&(binding.package.clone(), binding.asset.clone())];
        let model = loaded.animated_model().unwrap();
        assert_eq!(model.to_bytes().unwrap(), bytes);
        assert_payload(model, quarter);
        let counts = orr_sample::room_view::check_model_poses(
            orr_bridge::FrameView::of(prepared.scene().frame()),
            prepared.scene().index(),
            prepared.models(),
            None,
        )
        .unwrap();
        assert!(counts.animated_models > 0);
        let placed = orr_sample::room_view::character_placement(
            orr_bridge::FrameView::of(prepared.scene().frame()),
            prepared.scene().index(),
            prepared.models(),
            orr_sample::room_view::PresentationTime::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(placed.entity, player);
        assert_payload(&placed.model, quarter);
        let original = orr_model::animation::AnimatedModel::from_bytes(&source_bytes()).unwrap();
        let source_pose = original.sample_clip(0, 0.).unwrap();
        assert_eq!(placed.pose.local(), source_pose.local());
        assert_eq!(placed.pose.skin_matrices(), source_pose.skin_matrices());
        assert_deformed(
            &original.deform(&source_pose).unwrap(),
            &placed.model.deform(&placed.pose).unwrap(),
            quarter,
        );
        let baseline = crate::package_manager::presentation_tests::fixture(true, true);
        assert_eq!(
            prepared.scene().frame().to_bytes(),
            open(&baseline.root).scene().frame().to_bytes(),
            "presentation variant cannot alter authored simulation"
        );
    }
}
// Independent scale -> positive quarter-turn -> offset oracle.
fn transformed([u, v]: [f32; 2], quarter: u32) -> [f32; 2] {
    let (u, v) = (u * 2., v * 0.5);
    match quarter {
        1 => [0.25 - v, -0.5 + u],
        3 => [0.25 + v, -0.5 - u],
        _ => panic!("fixture rotation"),
    }
}
fn assert_deformed(
    original: &[orr_model::Primitive],
    actual: &[orr_model::Primitive],
    quarter: u32,
) {
    assert_eq!(original.len(), actual.len());
    let mut selected = 0;
    let mut other = 0;
    for (a, b) in original.iter().zip(actual) {
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.material, b.material);
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (a_vertex, b_vertex) in a.vertices.iter().zip(&b.vertices) {
            assert_eq!(a_vertex.position, b_vertex.position);
            assert_eq!(a_vertex.normal, b_vertex.normal);
            if a.material == material_index() {
                assert_eq!(b_vertex.uv, transformed(a_vertex.uv, quarter));
                selected += 1;
            } else {
                assert_eq!(a_vertex.uv.map(f32::to_bits), b_vertex.uv.map(f32::to_bits));
                other += 1;
            }
        }
    }
    assert!(
        selected > 0 && other > 0,
        "both selected and shared-image other material must be exercised"
    );
}
fn assert_payload(model: &orr_model::animation::AnimatedModel, quarter: u32) {
    let old = orr_model::animation::AnimatedModel::from_bytes(&source_bytes()).unwrap();
    let before = old.source();
    let after = model.source();
    assert_eq!(after.asset_id, "variants/courier.uv.json");
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.images, before.images);
    assert_eq!(
        after.materials[material_index() as usize].image,
        after.materials[0].image
    );
    let mut restored = after.clone();
    restored.asset_id = before.asset_id.clone();
    restored.dependencies = before.dependencies.clone();
    for (a, b) in before.primitives.iter().zip(&mut restored.primitives) {
        b.id = a.id.clone();
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (av, bv) in a.vertices.iter().zip(&mut b.vertices) {
            let expected = if a.material == material_index() {
                transformed(av.vertex.uv, quarter)
            } else {
                assert_eq!(
                    bv.vertex.uv.map(f32::to_bits),
                    av.vertex.uv.map(f32::to_bits)
                );
                av.vertex.uv
            };
            assert_eq!(bv.vertex.uv, expected);
            bv.vertex.uv = av.vertex.uv;
        }
    }
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(before).unwrap(),
        "only selected material UVs and identity/provenance may change"
    );
    for clip in 0..before.clips.len() as u32 {
        for time in [0., 0.25, 0.75] {
            let a = old.sample_clip(clip, time).unwrap();
            let b = model.sample_clip(clip, time).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
            assert_deformed(
                &old.deform(&a).unwrap(),
                &model.deform(&b).unwrap(),
                quarter,
            );
        }
    }
}
