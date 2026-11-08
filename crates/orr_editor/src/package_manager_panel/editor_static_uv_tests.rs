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
const VARIANT_ID: &str = "variants/prop.uv.json";
const PACKAGE: &str = "imported-static-uv";
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
                [0.0, y, 0.1, 1.0],
            ];
            if primitive.material == 1 {
                primitive.vertices[0].uv = [-0.0, 0.0];
            }
            source.primitives.push(primitive);
        }
    }
    source.materials[0].base_color[3] = 0.25;
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Repeat;
    source.materials[1].base_color[3] = 0.75;
    source.materials[1].image = 0;
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

fn import_static(
    quarter: Option<u32>,
) -> (
    crate::package_manager::presentation_tests::Fixture,
    crate::model_package_import::Report,
    Vec<u8>,
) {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let source = fixture.temp.path().join("author.model.json");
    let original = source_model().to_bytes().unwrap();
    fs::write(&source, &original).unwrap();
    let license = fixture.temp.path().join("author-LICENSE.txt");
    let license_bytes = b"Acceptance fixture: redistribution acknowledged";
    fs::write(&license, license_bytes).unwrap();
    let destination = fixture.temp.path().join("new static package");
    let before = snapshot(&fixture.root);
    let mut h = Harness::builder()
        .with_size([1400.0, 1800.0])
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
    if let Some(quarter) = quarter {
        click(&mut h, "Create static UV variant");
        assert!(h
            .get_by_label("Import cooked static package")
            .accesskit_node()
            .is_disabled());
        let inspection_files = snapshot(fixture.temp.path());
        click(&mut h, "Inspect static source");
        wait_import(&mut h);
        assert_eq!(
            snapshot(fixture.temp.path()),
            inspection_files,
            "inspection is read-only"
        );
        click(&mut h, "Use material 0");
        for (label, value) in [
            ("Static UV offset U", "0.25"),
            ("Static UV offset V", "-0.5"),
            ("Static UV scale U", "2"),
            ("Static UV scale V", "0.5"),
            ("Static UV variant asset ID", VARIANT_ID),
        ] {
            text(&mut h, label, value);
        }
        click(&mut h, "Static UV quarter-turn rotation");
        click(&mut h, &format!("{} degrees", quarter * 90));
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
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "form edits cannot change a project"
    );
    click(&mut h, RIGHTS);
    assert!(!h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Import cooked static package");
    wait_import(&mut h);
    let report = h
        .state_mut()
        .2
        .take()
        .expect("worker returned current immutable request");
    assert_eq!(report.root, destination);
    assert_eq!(report.package, PACKAGE);
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.kind, ModelKind::Static);
    assert_eq!(report.license_sha256, digest(license_bytes));
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        license_bytes
    );
    let payload = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(report.source_sha256, digest(&payload));
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&license).unwrap(), license_bytes);
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "import publishes a separate package only"
    );
    assert_eq!(
        h.get_by_label(RIGHTS).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    (fixture, report, payload)
}

// Independent equations: scale first, turn around (0,0), then translate.
fn expected_uv([u, v]: [f32; 2], quarter: u32) -> [f32; 2] {
    match quarter {
        1 => [0.25 - 0.5 * v, -0.5 + 2.0 * u],
        3 => [0.25 + 0.5 * v, -0.5 - 2.0 * u],
        _ => panic!("fixture quarter-turn"),
    }
}

fn assert_variant(actual: &StaticModel, quarter: u32) {
    let original = source_model();
    let before = original.source();
    let after = actual.source();
    assert_eq!(after.asset_id, VARIANT_ID);
    assert_eq!(after.primitives.len(), 4);
    assert_eq!(after.materials.len(), 3);
    assert_eq!(after.images.len(), 3);
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.images, before.images);
    assert_eq!(after.materials[0].image, after.materials[1].image);
    let mut selected = 0;
    let mut unselected = 0;
    for (old, new) in before.primitives.iter().zip(&after.primitives) {
        let mut expected = old.clone();
        expected.id = format!("{}{}", VARIANT_ID, old.id.strip_prefix(SOURCE_ID).unwrap());
        for (old_vertex, expected_vertex) in old.vertices.iter().zip(&mut expected.vertices) {
            if old.material == 0 {
                expected_vertex.uv = expected_uv(old_vertex.uv, quarter);
            }
        }
        assert_eq!(
            new, &expected,
            "positions, normals, indices and transforms must survive"
        );
        if old.material == 0 {
            selected += 1;
        } else {
            unselected += 1;
            for (old, new) in old.vertices.iter().zip(&new.vertices) {
                assert_eq!(new.uv.map(f32::to_bits), old.uv.map(f32::to_bits));
            }
        }
    }
    assert_eq!((selected, unselected), (2, 2));
    assert_eq!(after.dependencies.len(), 2);
    assert_eq!(after.dependencies[0].uri, "$source");
    assert_eq!(after.dependencies[1].uri, "input.model.json");
    assert_eq!(
        after.dependencies[1].sha256,
        digest(&original.to_bytes().unwrap())
    );
}

#[test]
fn actual_static_uv_widgets_import_install_binding_history_save_reopen_and_cpu_geometry() {
    for quarter in [1, 3] {
        let (fixture, report, bytes) = import_static(Some(quarter));
        assert_variant(&StaticModel::from_bytes(&bytes).unwrap(), quarter);
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
            "Preview static model imported-static-uv/model.orrmodel",
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
        assert_variant(
            preview.ready().unwrap().loaded.static_model().unwrap(),
            quarter,
        );
        click(&mut h, "Use model imported-static-uv/model.orrmodel");
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
            "binding UV edits preserve world placement"
        );
        placed.instance.validate_for(&placed.model).unwrap();
        assert_eq!(placed.model.to_bytes().unwrap(), bytes);
        assert_variant(&placed.model, quarter);
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
    }
}

#[test]
fn unchecked_static_uv_preserves_exact_cooked_import_and_original_project() {
    let (fixture, report, bytes) = import_static(None);
    assert_eq!(bytes, source_model().to_bytes().unwrap());
    assert_eq!(fs::read(report.root.join("model.orrmodel")).unwrap(), bytes);
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
    let baseline = crate::package_manager::presentation_tests::fixture(false, false);
    let opened = orr_sample::room_project::PreparedProject::open(&fixture.root).unwrap();
    let original = orr_sample::room_project::PreparedProject::open(&baseline.root).unwrap();
    assert_eq!(
        opened.scene().frame().to_bytes(),
        original.scene().frame().to_bytes()
    );
    assert_eq!(opened.models().document, original.models().document);
}
