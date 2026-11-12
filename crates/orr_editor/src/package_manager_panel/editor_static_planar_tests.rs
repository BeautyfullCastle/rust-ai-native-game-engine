//! CPU acceptance: real import/package widgets, then validated binding transactions.
//! Assignment/history/save use Bindings' production API, not simulated UI clicks.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_model::{Dependency, Image, StaticModel, Wrap};
use sha2::{Digest, Sha256};
use std::{
    fs,
    time::{Duration, Instant},
};

const GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const SOURCE_ID: &str = "scene/source.glb";
const VARIANT_ID: &str = "variants/prop.planar-uv.json";
const PACKAGE: &str = "imported-static-planar";
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

// Two users per material, mirrored nodes, a shared image and an unused slot.
// This deliberately follows the installed static-model consumer's fixture.
fn source_model() -> StaticModel {
    let imported = orr_model::import::import_with_resolver(SOURCE_ID, GLB, |_| {
        panic!("embedded GLB must not resolve dependencies")
    })
    .unwrap();
    let mut source = imported.source().clone();
    let primitives = std::mem::take(&mut source.primitives);
    for (node, sign, y) in [(1, -1.0, 1.25), (7, 1.0, -1.25)] {
        for (slot, primitive) in primitives.iter().enumerate() {
            let mut primitive = primitive.clone();
            primitive.id = format!("{SOURCE_ID}#node={node}/mesh=0/primitive={slot}");
            primitive.transform = [
                [0.75 * sign, 0.0, 0.0, 0.0],
                [0.0, 0.75, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, y, 0.125, 1.0],
            ];
            primitive.transform[0][1] = -0.0;
            primitive.vertices[0].normal = [0.6, -0.0, 0.8];
            primitive.vertices[0].position[2] = -0.0;
            if primitive.material == 0 {
                primitive.vertices[0].uv = [-0.0, 0.0];
            }
            source.primitives.push(primitive);
        }
    }
    source.materials[0].base_color[3] = 0.25;
    source.materials[0].linear_filter = false;
    source.materials[0].wrap_s = Wrap::Repeat;
    source.materials[0].wrap_t = Wrap::Mirror;
    source.materials[1].base_color[3] = 0.75;
    source.materials[1].image = 0;
    source.materials[1].linear_filter = true;
    source.materials[1].wrap_s = Wrap::Clamp;
    source.materials[1].wrap_t = Wrap::Repeat;
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![17, 31, 47, 63],
    });
    let mut unused = source.materials[0].clone();
    unused.image = 2;
    unused.base_color = [0.25, 0.5, 0.75, 0.125];
    unused.linear_filter = true;
    source.materials.push(unused);
    source.dependencies.push(Dependency {
        uri: "provenance/never-opened.txt".into(),
        sha256: digest(b"historical source metadata"),
    });
    StaticModel::new(source).unwrap()
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
    plane: &str,
) -> Harness<'static, ImportState> {
    let source = fixture.temp.path().join("author.model.json");
    fs::write(&source, source_model().to_bytes().unwrap()).unwrap();
    let license = fixture.temp.path().join("author-LICENSE.txt");
    fs::write(&license, b"Acceptance fixture: redistribution acknowledged").unwrap();
    let destination = fixture.temp.path().join("new static planar package");
    let before = snapshot(&fixture.root);
    let mut h = Harness::builder()
        .with_size([1400.0, 2400.0])
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
    click(&mut h, "Cooked static model");
    assert_eq!(
        h.get_by_label("Create static planar UV variant")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    for (label, value) in [
        (
            "Cooked static model file (.model.json)",
            source.to_str().unwrap(),
        ),
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
    click(&mut h, "Create static planar UV variant");
    assert!(h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    let inspection_files = snapshot(fixture.temp.path());
    click(&mut h, "Inspect static source");
    wait_import(&mut h);
    assert_eq!(snapshot(fixture.temp.path()), inspection_files);
    assert!(
        h.get_by_label("Import cooked static package")
            .accesskit_node()
            .is_disabled(),
        "inspection cannot auto-select a material"
    );
    click(&mut h, "Use material 1");
    click(&mut h, "Static planar UV plane");
    click(
        &mut h,
        match plane {
            "xy" => "XY model plane",
            "xz" => "XZ model plane",
            "yz" => "YZ model plane",
            _ => panic!("fixture plane"),
        },
    );
    for (label, value) in [
        ("Planar origin X", "0.25"),
        ("Planar origin Y", "-0.5"),
        ("Planar origin Z", "1.25"),
        ("Planar units per repeat U", "2"),
        ("Planar units per repeat V", "0.5"),
        ("Static planar UV asset ID", VARIANT_ID),
    ] {
        text(&mut h, label, value);
    }
    assert!(h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        h.get_by_label(RIGHTS).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    assert!(!destination.exists());
    assert_eq!(snapshot(&fixture.root), before);
    click(&mut h, RIGHTS);
    assert!(!h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    h
}
fn assert_inputs(fixture: &crate::package_manager::presentation_tests::Fixture) {
    assert_eq!(
        fs::read(fixture.temp.path().join("author.model.json")).unwrap(),
        source_model().to_bytes().unwrap()
    );
    assert_eq!(
        fs::read(fixture.temp.path().join("author-LICENSE.txt")).unwrap(),
        b"Acceptance fixture: redistribution acknowledged"
    );
}
// Independent expansion of source node transforms and projection parameters.
// Intermediate arithmetic is f64; f32 narrowing occurs only at the final UV.
fn expected_uv(index: usize, [x, y, z]: [f32; 3], plane: &str) -> [f32; 2] {
    let (sign, origin_y) = if index < 2 {
        (-1.0, 1.25)
    } else {
        (1.0, -1.25)
    };
    let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
    let uv = match plane {
        "xy" => [0.375 * sign * x - 0.125, 1.5 * y + 2.0 * origin_y + 1.0],
        "xz" => [0.375 * sign * x - 0.125, 2.0 * z - 2.25],
        "yz" => [0.375 * y + 0.5 * origin_y + 0.25, 2.0 * z - 2.25],
        _ => panic!("fixture plane"),
    };
    uv.map(|value| value as f32)
}
fn assert_planar(model: &StaticModel, plane: &str) {
    let source = source_model();
    let before = source.source();
    let after = model.source();
    assert_eq!(after.asset_id, VARIANT_ID);
    assert_eq!(after.primitives.len(), 4);
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.images, before.images);
    assert_eq!(after.materials[0].image, after.materials[1].image);
    let mut restored = after.clone();
    restored.asset_id = before.asset_id.clone();
    restored.dependencies = before.dependencies.clone();
    let mut selected = 0;
    let mut other = 0;
    for (index, (old, new)) in before
        .primitives
        .iter()
        .zip(&mut restored.primitives)
        .enumerate()
    {
        assert_eq!(
            new.id,
            format!("{}{}", VARIANT_ID, old.id.strip_prefix(SOURCE_ID).unwrap())
        );
        new.id = old.id.clone();
        assert_eq!(
            new.transform.map(|c| c.map(f32::to_bits)),
            old.transform.map(|c| c.map(f32::to_bits))
        );
        assert_eq!(new.vertices.len(), old.vertices.len());
        for (old_vertex, new_vertex) in old.vertices.iter().zip(&mut new.vertices) {
            assert_eq!(
                new_vertex.position.map(f32::to_bits),
                old_vertex.position.map(f32::to_bits)
            );
            assert_eq!(
                new_vertex.normal.map(f32::to_bits),
                old_vertex.normal.map(f32::to_bits)
            );
            if old.material == 1 {
                assert_eq!(
                    new_vertex.uv.map(f32::to_bits),
                    expected_uv(index, old_vertex.position, plane).map(f32::to_bits)
                );
                new_vertex.uv = old_vertex.uv;
            } else {
                assert_eq!(
                    new_vertex.uv.map(f32::to_bits),
                    old_vertex.uv.map(f32::to_bits)
                );
            }
        }
        if old.material == 1 {
            selected += 1;
        } else {
            other += 1;
        }
    }
    assert_eq!((selected, other), (2, 2));
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(before).unwrap(),
        "only selected material UVs and identity/provenance may change"
    );
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format":"orr_static_planar_uv","version":1,"source":"input.model.json","material":1,
        "origin":[0.25_f64,-0.5,1.25],"units_per_repeat":[2.0_f64,0.5],"plane":plane,
    }))
    .unwrap();
    assert_eq!(
        after.dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(&recipe)
            },
            Dependency {
                uri: "input.model.json".into(),
                sha256: digest(&source.to_bytes().unwrap())
            },
        ]
    );
}

#[test]
fn actual_static_planar_widgets_all_planes_install_key_history_save_reopen_and_room_cpu_uv() {
    for plane in ["xy", "xz", "yz"] {
        let fixture = crate::package_manager::presentation_tests::fixture(false, false);
        let mut import_ui = form(&fixture, plane);
        let before = snapshot(&fixture.root);
        click(&mut import_ui, "Import cooked static package");
        wait_import(&mut import_ui);
        let report = import_ui
            .state_mut()
            .2
            .take()
            .expect("current worker import");
        assert_eq!(
            report.root,
            fixture.temp.path().join("new static planar package")
        );
        assert_eq!(report.package, PACKAGE);
        assert_eq!(report.kind, ModelKind::Static);
        assert_eq!(report.asset, "model.orrmodel");
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(report.root.join("orr.package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["capabilities"], serde_json::json!(["models"]));
        let bytes = fs::read(report.root.join(&report.asset)).unwrap();
        assert_eq!(report.source_sha256, digest(&bytes));
        let license = b"Acceptance fixture: redistribution acknowledged";
        assert_eq!(report.license_sha256, digest(license));
        assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
        assert_planar(&StaticModel::from_bytes(&bytes).unwrap(), plane);
        assert_inputs(&fixture);
        assert_eq!(
            snapshot(&fixture.root),
            before,
            "import publishes a separate immutable package"
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
                        .is_some_and(|actor| actor.kind == orr_sample::room_game::KEY)
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
        assert_eq!(lock.packages[PACKAGE].files.len(), 2);
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
            "Preview static model imported-static-planar/model.orrmodel",
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
        assert_planar(
            preview.ready().unwrap().loaded.static_model().unwrap(),
            plane,
        );
        click(&mut h, "Use model imported-static-planar/model.orrmodel");
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
            "planar UV authoring preserves the KEY binding placement"
        );
        placed.instance.validate_for(&placed.model).unwrap();
        assert_eq!(placed.model.to_bytes().unwrap(), bytes);
        assert_planar(&placed.model, plane);
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
            fs::read(fixture.temp.path().join("author.model.json")).unwrap(),
            source_model().to_bytes().unwrap()
        );
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
        assert_inputs(&fixture);
    }
}

#[test]
fn changed_inspected_planar_source_rejects_without_publication_or_room_changes() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let mut h = form(&fixture, "yz");
    let source = fixture.temp.path().join("author.model.json");
    let mut changed = fs::read(&source).unwrap();
    changed.push(b'\n');
    fs::write(&source, &changed).unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, "Import cooked static package");
    wait_import(&mut h);
    assert!(h.state().2.is_none());
    let status = h.state().0.status().to_ascii_lowercase();
    assert!(
        status.contains("failed") && (status.contains("changed") || status.contains("hash")),
        "{status}"
    );
    assert!(!fixture
        .temp
        .path()
        .join("new static planar package")
        .exists());
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "stale source cannot publish or edit Room"
    );
    assert_eq!(fs::read(source).unwrap(), changed);
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
}
