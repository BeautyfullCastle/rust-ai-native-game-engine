//! CPU acceptance: real import/package widgets, then validated binding transactions.
//! Assignment/history/save use Bindings' production API, not simulated UI clicks.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_model::{Dependency, StaticModel};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    time::{Duration, Instant},
};

const GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const SOURCE_ID: &str = "props/selected.glb";
const PACKAGE: &str = "imported-glb-subtree";
const IMPORT: &str = "Import GLB subtree package";
const RIGHTS: &str = "I have reviewed rights to distribute this content";
type ImportState = (
    crate::model_package_import_panel::Panel,
    PathBuf,
    Option<crate::model_package_import::Report>,
);

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// Same bounded embedded scene used by the extractor's integration fixtures.
// Two nodes intentionally share a name; node identity is the index.
fn parts(bytes: &[u8]) -> (Value, Vec<u8>) {
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    (
        serde_json::from_slice(&bytes[20..20 + length]).unwrap(),
        bytes[28 + length..].to_vec(),
    )
}
fn pack(document: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut output = b"glTF".to_vec();
    output.extend(2u32.to_le_bytes());
    output.extend(((28 + json.len() + binary.len()) as u32).to_le_bytes());
    output.extend((json.len() as u32).to_le_bytes());
    output.extend(b"JSON");
    output.extend(json);
    output.extend((binary.len() as u32).to_le_bytes());
    output.extend(b"BIN\0");
    output.extend(binary);
    output
}
fn fixture_parts() -> (Value, Vec<u8>) {
    let (mut doc, bin) = parts(GLB);
    doc["scene"] = json!(0);
    doc["nodes"] = json!([
        {"name":"assembly","translation":[3,2,1],"children":[1,2,4]},
        {"name":"prop","mesh":0,"translation":[1,0,0],"scale":[-2,1,1],"children":[3]},
        {"name":"excluded sibling","mesh":0,"translation":[-10,0,0]},
        {"name":"prop","mesh":0,"translation":[0,4,0],"scale":[1,2,1]},
        {"name":"empty"},
        {"name":"excluded other scene","mesh":0}
    ]);
    doc["scenes"] = json!([{"nodes":[0]},{"nodes":[5]}]);
    // Preserve an unused material and image, with two used slots sharing image0.
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] =
        json!([0.25, 0.5, 0.75, 0.125]);
    doc["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"]["index"] = json!(0);
    let unused = doc["materials"][0].clone();
    doc["materials"].as_array_mut().unwrap().push(unused);
    (doc, bin)
}
fn source_bytes() -> Vec<u8> {
    let (doc, bin) = fixture_parts();
    pack(&doc, &bin)
}
fn full_model() -> StaticModel {
    orr_model::import::import_with_resolver(SOURCE_ID, &source_bytes(), |_| {
        panic!("fixture must be fully embedded")
    })
    .unwrap()
}
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.push((path.strip_prefix(root).unwrap().into(), Vec::new()));
                visit(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

fn click<T>(h: &mut Harness<'_, T>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
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

fn wait_import(h: &mut Harness<'_, ImportState>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().0.busy() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn wait_packages(h: &mut Harness<'_, (Panel, PathBuf)>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().0.busy() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status);
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn form(
    fixture: &crate::package_manager::presentation_tests::Fixture,
    node: u32,
) -> Harness<'static, ImportState> {
    let source = fixture.temp.path().join("author.glb");
    fs::write(&source, source_bytes()).unwrap();
    let license = fixture.temp.path().join("author-LICENSE.txt");
    fs::write(&license, b"Acceptance fixture: redistribution acknowledged").unwrap();
    let destination = fixture.temp.path().join("new subtree package");
    let before = snapshot(&fixture.root);
    let mut h = Harness::builder()
        .with_size([1400.0, 2200.0])
        .build_ui_state(
            |ui, state: &mut ImportState| {
                if let Some(report) = state.0.poll(Some(&state.1), 4, 7) {
                    state.2 = Some(report);
                }
                state.0.show(ui, true, Some(&state.1), 4, 7);
            },
            (
                crate::model_package_import_panel::Panel::default(),
                fixture.root.clone(),
                None,
            ),
        );
    h.run_steps(3);
    click(&mut h, "Import a model package");
    click(&mut h, "Raw GLB");
    assert_eq!(
        h.get_by_label("Create static GLB subtree")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    for (label, value) in [
        ("Raw GLB file", source.to_str().unwrap()),
        ("License file", license.to_str().unwrap()),
        (
            "Package destination directory",
            destination.to_str().unwrap(),
        ),
        ("Package name", PACKAGE),
        ("Package version", "1.0.0"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Create static GLB subtree");
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "inspection and explicit selection required"
    );
    let inspection_files = snapshot(fixture.temp.path());
    click(&mut h, "Inspect GLB nodes");
    wait_import(&mut h);
    assert_eq!(
        snapshot(fixture.temp.path()),
        inspection_files,
        "node inspection is read-only"
    );
    for selectable in [0, 1, 2, 3] {
        assert!(!h
            .get_by_label(&format!("Use node {selectable}"))
            .accesskit_node()
            .is_disabled());
    }
    assert!(
        h.query_by_label("Use node 4").is_none(),
        "empty subtree is not selectable"
    );
    assert!(
        h.query_by_label("Use node 5").is_none(),
        "other scene is excluded"
    );
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "inspection alone cannot select"
    );
    click(&mut h, &format!("Use node {node}"));
    text(&mut h, "GLB subtree asset ID", SOURCE_ID);
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "selection cannot grant distribution consent"
    );
    assert_eq!(
        h.get_by_label(RIGHTS).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    assert!(!destination.exists());
    assert_eq!(snapshot(&fixture.root), before);
    click(&mut h, RIGHTS);
    assert!(!h.get_by_label(IMPORT).accesskit_node().is_disabled());
    h
}
fn assert_inputs(fixture: &crate::package_manager::presentation_tests::Fixture) {
    assert_eq!(
        fs::read(fixture.temp.path().join("author.glb")).unwrap(),
        source_bytes()
    );
    assert_eq!(
        fs::read(fixture.temp.path().join("author-LICENSE.txt")).unwrap(),
        b"Acceptance fixture: redistribution acknowledged"
    );
}

// Parent TRS composes before the selected object's mirrored/nonuniform TRS;
// all selected descendants retain ancestors. These matrices are hand expanded.
fn assert_subtree(actual: &StaticModel, node: u32) {
    let full = full_model();
    let before = full.source();
    let after = actual.source();
    assert_eq!(before.primitives.len(), 6);
    assert_eq!(after.asset_id, SOURCE_ID);
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.images, before.images);
    assert_eq!(after.materials.len(), 3);
    assert_eq!(after.images.len(), 2);
    assert_eq!(after.materials[0].image, after.materials[1].image);
    let expected = if node == 1 {
        &before.primitives[..4]
    } else {
        &before.primitives[2..4]
    };
    assert_eq!(
        after.primitives, expected,
        "exact selected subtree, including stable source order and geometry"
    );
    for (index, primitive) in after.primitives.iter().enumerate() {
        let nested = node == 3 || index >= 2;
        let expected_node = if nested { 3 } else { 1 };
        let height_scale = if nested { 2.0 } else { 1.0 };
        let origin_y = if nested { 6.0 } else { 2.0 };
        let matrix = [
            [-2.0, 0.0, 0.0, 0.0],
            [0.0, height_scale, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [4.0, origin_y, 1.0, 1.0],
        ];
        assert_eq!(
            primitive.transform, matrix,
            "selection never recenters or drops ancestor transforms"
        );
        assert_eq!(
            primitive.id,
            format!(
                "{SOURCE_ID}#node={expected_node}/mesh=0/primitive={}",
                index % 2
            )
        );
        assert_eq!(primitive.material, (index % 2) as u32);
        let normals = orr_model::normal_matrix(primitive.transform).unwrap();
        for vertex in &primitive.vertices {
            let [x, y, z] = vertex.position;
            let actual_position: [f32; 3] = std::array::from_fn(|axis| {
                primitive.transform[0][axis] * x
                    + primitive.transform[1][axis] * y
                    + primitive.transform[2][axis] * z
                    + primitive.transform[3][axis]
            });
            assert_eq!(
                actual_position,
                [4.0 - 2.0 * x, origin_y + height_scale * y, 1.0 + z]
            );
            let [x, y, z] = vertex.normal;
            let actual_normal: [f32; 3] = std::array::from_fn(|axis| {
                normals[0][axis] * x + normals[1][axis] * y + normals[2][axis] * z
            });
            assert_eq!(actual_normal, [-0.5 * x, y / height_scale, z]);
        }
    }
    assert_eq!(
        after.dependencies,
        vec![Dependency {
            uri: "$source".into(),
            sha256: digest(&source_bytes())
        }]
    );
}
#[test]
fn actual_glb_subtree_widgets_import_install_binding_history_save_reopen_and_cpu_geometry() {
    for node in [1, 3] {
        let fixture = crate::package_manager::presentation_tests::fixture(false, false);
        let mut import_ui = form(&fixture, node);
        let before_import = snapshot(&fixture.root);
        click(&mut import_ui, IMPORT);
        wait_import(&mut import_ui);
        let report = import_ui
            .state_mut()
            .2
            .take()
            .expect("worker returned current request");
        assert_eq!(report.asset, "model.orrmodel");
        assert_eq!(report.package, PACKAGE);
        assert_eq!(report.kind, ModelKind::Static);
        assert_eq!(report.root, fixture.temp.path().join("new subtree package"));
        let bytes = fs::read(report.root.join(&report.asset)).unwrap();
        assert_eq!(report.source_sha256, digest(&bytes));
        let license = b"Acceptance fixture: redistribution acknowledged";
        assert_eq!(report.license_sha256, digest(license));
        assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
        assert_subtree(&StaticModel::from_bytes(&bytes).unwrap(), node);
        assert_inputs(&fixture);
        assert_eq!(
            snapshot(&fixture.root),
            before_import,
            "import publishes a separate package only"
        );
        assert_eq!(
            import_ui.get_by_label(RIGHTS).accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        let initial = orr_sample::room_project::PreparedProject::open(&fixture.root).unwrap();
        let frame_bytes = initial.scene().frame().to_bytes();
        let (guid, old_binding) = initial
            .models()
            .document
            .bindings
            .iter()
            .find(|(guid, binding)| {
                let entity = initial
                    .scene()
                    .index()
                    .entity(&orr_reflect::Guid::parse(guid).unwrap())
                    .unwrap();
                binding.kind == ModelKind::Static
                    && initial
                        .scene()
                        .frame()
                        .get::<orr_sample::room_game::RoomActor>(entity)
                        .is_some_and(|actor| actor.kind != orr_sample::room_game::PLAYER)
            })
            .map(|(guid, binding)| (orr_reflect::Guid::parse(guid).unwrap(), binding.clone()))
            .unwrap();
        let entity = initial.scene().index().entity(&guid).unwrap();
        let old_instances = orr_sample::room_view::placements(
            orr_bridge::FrameView::of(initial.scene().frame()),
            initial.scene().index(),
            initial.models(),
        )
        .unwrap();
        let old_instance = old_instances
            .iter()
            .find(|p| p.entity == entity)
            .unwrap()
            .instance;
        let mut bindings =
            crate::model_bindings::Bindings::open(fixture.root.join("room.models.json")).unwrap();
        let initial_document = bindings.document().clone();
        assert!(!bindings.dirty());
        let project_before_install = snapshot(&fixture.root);
        let old_objects = snapshot(&fixture.root.join(".orr/packages/objects"));
        let mut h = Harness::builder()
            .with_size([1500.0, 1800.0])
            .build_ui_state(
                |ui, state: &mut (Panel, PathBuf)| state.0.show(ui.ctx(), Some(&state.1), None),
                (
                    Panel {
                        open: true,
                        source: report.root.display().to_string(),
                        ..Default::default()
                    },
                    fixture.root.clone(),
                ),
            );
        h.run_steps(3);
        click(&mut h, "Preview package addition");
        wait_packages(&mut h);
        assert!(h.state().0.preview.is_some(), "{}", h.state().0.status);
        assert_eq!(
            snapshot(&fixture.root),
            project_before_install,
            "package preview cannot install"
        );
        click(&mut h, "Add previewed package");
        wait_packages(&mut h);
        click(&mut h, "Verify installed packages");
        wait_packages(&mut h);
        let lock = h.state().0.verified.as_ref().unwrap();
        assert!(lock.direct.contains_key(PACKAGE));
        assert_eq!(
            lock.packages[PACKAGE].files.len(),
            2,
            "only cooked subtree and license are installed"
        );
        assert_eq!(lock.packages[PACKAGE].files[&report.asset], digest(&bytes));
        let installed_before_assign = snapshot(&fixture.root.join(".orr/packages/objects"));
        for old in old_objects {
            assert!(
                installed_before_assign.contains(&old),
                "existing installed bytes cannot change"
            );
        }
        click(
            &mut h,
            "Preview static model imported-glb-subtree/model.orrmodel",
        );
        let request = h.state_mut().0.take_preview_request().unwrap();
        assert_eq!(request.kind, ModelKind::Static);
        let mut preview = crate::package_model_preview::State::default();
        preview.request(request);
        let deadline = Instant::now() + Duration::from_secs(10);
        while preview.ready().is_none() {
            preview.poll(Some(&fixture.root));
            assert!(Instant::now() < deadline, "{}", preview.status);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_subtree(
            preview.ready().unwrap().loaded.static_model().unwrap(),
            node,
        );
        click(&mut h, "Use model imported-glb-subtree/model.orrmodel");
        let (package, asset, kind) = h.state_mut().0.take_model_asset_request().unwrap();
        assert_eq!(
            (&package, &asset, kind),
            (&report.package, &report.asset, ModelKind::Static)
        );
        assert_eq!(bindings.document(), &initial_document);
        bindings.undo();
        assert_eq!(
            bindings.document(),
            &initial_document,
            "import/install create no binding history"
        );
        assert!(!bindings.dirty());
        // Production validated transaction used by Assign; this is an API assertion.
        let loaded =
            crate::model_bindings::load_asset_for_kind(&fixture.root, &package, &asset, kind)
                .unwrap();
        let binding = crate::model_bindings::Binding::from_asset(
            package,
            asset,
            &loaded,
            old_binding.transform,
        )
        .unwrap();
        bindings
            .assign_validated(std::slice::from_ref(&guid), &binding, &loaded)
            .unwrap();
        assert!(bindings.dirty());
        let assigned = bindings.document().clone();
        bindings.undo();
        assert_eq!(bindings.document(), &initial_document);
        assert!(!bindings.dirty());
        bindings.redo();
        assert_eq!(bindings.document(), &assigned);
        assert!(bindings.dirty());
        bindings.save().unwrap();
        assert!(!bindings.dirty());
        let reopened_bindings =
            crate::model_bindings::Bindings::open(fixture.root.join("room.models.json")).unwrap();
        assert_eq!(reopened_bindings.document(), &assigned);
        assert!(!reopened_bindings.dirty());
        let reopened = orr_sample::room_project::PreparedProject::open(&fixture.root).unwrap();
        assert_eq!(reopened.scene().frame().to_bytes(), frame_bytes);
        assert_eq!(
            reopened.models().document.bindings[&guid.to_string()],
            binding
        );
        let consumer = orr_sample::room_view::placements(
            orr_bridge::FrameView::of(reopened.scene().frame()),
            reopened.scene().index(),
            reopened.models(),
        )
        .unwrap();
        let placed = consumer.iter().find(|p| p.entity == entity).unwrap();
        assert_eq!(
            placed.instance, old_instance,
            "selected subtree preserves the binding world placement"
        );
        placed.instance.validate_for(&placed.model).unwrap();
        assert_eq!(placed.model.to_bytes().unwrap(), bytes);
        assert_subtree(&placed.model, node);
        assert_eq!(
            reopened.scene().frame().to_bytes(),
            frame_bytes,
            "CPU consumer cannot write Frame"
        );
        assert_eq!(
            snapshot(&fixture.root.join(".orr/packages/objects")),
            installed_before_assign
        );
        for (path, data) in project_before_install {
            if path != Path::new("room.models.json") && path != Path::new("orr.packages.lock.json")
            {
                let saved = fixture.root.join(&path);
                assert!(saved.exists(), "unrelated project path {path:?}");
                if saved.is_file() {
                    assert_eq!(
                        fs::read(saved).unwrap(),
                        data,
                        "unrelated project file {path:?}"
                    );
                } else {
                    assert!(
                        saved.is_dir() && data.is_empty(),
                        "unrelated directory {path:?}"
                    );
                }
            }
        }
        assert_eq!(
            fs::read(fixture.temp.path().join("author.glb")).unwrap(),
            source_bytes()
        );
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
        assert_inputs(&fixture);
    }
}

#[test]
fn changed_inspected_glb_source_rejects_import_without_publishing_or_editing_room() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let mut h = form(&fixture, 1);
    let (mut doc, bin) = fixture_parts();
    doc["nodes"][1]["name"] = json!("source renamed after inspection");
    let changed = pack(&doc, &bin);
    fs::write(fixture.temp.path().join("author.glb"), &changed).unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, IMPORT);
    wait_import(&mut h);
    assert!(h.state().2.is_none());
    let status = h.state().0.status().to_ascii_lowercase();
    assert!(
        status.contains("failed") && (status.contains("changed") || status.contains("hash")),
        "{status}"
    );
    assert!(!fixture.temp.path().join("new subtree package").exists());
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "checked hash failure cannot publish a package or mutate Room"
    );
    assert_eq!(
        fs::read(fixture.temp.path().join("author.glb")).unwrap(),
        changed
    );
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
}
