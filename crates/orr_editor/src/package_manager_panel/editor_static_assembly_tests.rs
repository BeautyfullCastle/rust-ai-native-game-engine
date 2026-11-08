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
const VARIANT_ID: &str = "variants/prop.assemble.json";
const PACKAGE: &str = "imported-static-assembly";
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
            primitive.transform[0][1] = -0.0;
            primitive.vertices[0].normal = [0.6, 0.0, 0.8];
            if primitive.material == 0 {
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

fn form(
    fixture: &crate::package_manager::presentation_tests::Fixture,
) -> Harness<'static, ImportState> {
    let before = snapshot(&fixture.root);
    let source = fixture.temp.path().join("author.model.json");
    fs::write(&source, source_model().to_bytes().unwrap()).unwrap();
    let license = fixture.temp.path().join("author-LICENSE.txt");
    fs::write(&license, b"Acceptance fixture: redistribution acknowledged").unwrap();
    let destination = fixture.temp.path().join("new static assembly package");
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
    assert_eq!(
        h.get_by_label("Create static two-copy assembly")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False),
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
    click(&mut h, "Create static two-copy assembly");
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
    assert!(h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Use inspected source for assembly");
    // Explicit identity first part must preserve original source matrix bits.
    for (part, offset, scale) in [
        (1, ["0", "0", "0"], ["1", "1", "1"]),
        (2, ["3", "1", "-2"], ["2", "1", "0.5"]),
    ] {
        for (axis, offset, scale) in [
            ("X", offset[0], scale[0]),
            ("Y", offset[1], scale[1]),
            ("Z", offset[2], scale[2]),
        ] {
            text(
                &mut h,
                &format!("Assembly part {part} offset {axis}"),
                offset,
            );
            text(&mut h, &format!("Assembly part {part} scale {axis}"), scale);
        }
    }
    click(&mut h, "Assembly part 2 yaw");
    click(&mut h, "90 degrees");
    text(&mut h, "Static assembly variant asset ID", VARIANT_ID);
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
        "form edits cannot change the project"
    );
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

// Expanded from the chosen source fixture and scale -> +Y yaw90 -> offset.
// No producer composition function or editor placement constructor is used.
fn expected_point(index: usize, part: usize, [x, y, z]: [f32; 3]) -> [f32; 3] {
    let (sign, origin_y) = if index < 2 {
        (-1.0, 1.25)
    } else {
        (1.0, -1.25)
    };
    let source = [0.75 * sign * x, 0.75 * y + origin_y, z + 0.1];
    if part == 0 {
        source
    } else {
        [
            3.0 + 0.5 * source[2],
            1.0 + source[1],
            -2.0 - 2.0 * source[0],
        ]
    }
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = v.iter().map(|v| v * v).sum::<f32>().sqrt();
    v.map(|v| v / len)
}
fn expected_normal(index: usize, part: usize, [x, y, z]: [f32; 3]) -> [f32; 3] {
    let sign = if index < 2 { -1.0 } else { 1.0 };
    let source = [x / (0.75 * sign), y / 0.75, z];
    unit(if part == 0 {
        source
    } else {
        [2.0 * source[2], source[1], -0.5 * source[0]]
    })
}
fn vector(m: [[f32; 4]; 4], v: [f32; 3], point: bool) -> [f32; 3] {
    std::array::from_fn(|axis| {
        m[0][axis] * v[0]
            + m[1][axis] * v[1]
            + m[2][axis] * v[2]
            + if point { m[3][axis] } else { 0.0 }
    })
}
fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "actual {actual} != expected {expected}"
        );
    }
}
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let xyz = [q[0], q[1], q[2]];
    let first = cross(xyz, v);
    let second = cross(xyz, first);
    std::array::from_fn(|axis| v[axis] + 2.0 * (q[3] * first[axis] + second[axis]))
}
fn instance_point(instance: &orr_render::StaticInstance, point: [f32; 3]) -> [f32; 3] {
    let scaled = std::array::from_fn(|axis| point[axis] * instance.scale[axis]);
    let rotated = rotate(instance.rotation, scaled);
    std::array::from_fn(|axis| rotated[axis] + instance.translation[axis])
}
fn instance_normal(instance: &orr_render::StaticInstance, normal: [f32; 3]) -> [f32; 3] {
    unit(rotate(
        instance.rotation,
        std::array::from_fn(|axis| normal[axis] / instance.scale[axis]),
    ))
}
fn geometry_oracle(
    model: &StaticModel,
    instance: Option<&orr_render::StaticInstance>,
) -> crate::package_model_preview::PreviewBounds {
    let original = source_model();
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for (index, actual) in model.source().primitives.iter().enumerate() {
        let part = index / 4;
        let source_index = index % 4;
        let source = &original.source().primitives[source_index];
        let normals = orr_model::normal_matrix(actual.transform).unwrap();
        for (old, new) in source.vertices.iter().zip(&actual.vertices) {
            let mut expected_position = expected_point(source_index, part, old.position);
            let mut actual_position = vector(actual.transform, new.position, true);
            let mut expected_direction = expected_normal(source_index, part, old.normal);
            let mut actual_direction = unit(vector(normals, new.normal, false));
            if let Some(instance) = instance {
                actual_position = instance_point(instance, actual_position);
                expected_position = instance_point(instance, expected_position);
                actual_direction = instance_normal(instance, actual_direction);
                expected_direction = instance_normal(instance, expected_direction);
            }
            near(actual_position, expected_position);
            near(actual_direction, expected_direction);
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(expected_position[axis]);
                maximum[axis] = maximum[axis].max(expected_position[axis]);
            }
        }
    }
    let center = std::array::from_fn(|axis| (minimum[axis] + maximum[axis]) * 0.5);
    let radius = minimum
        .iter()
        .zip(maximum)
        .map(|(min, max)| ((max - min) * 0.5).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(0.125);
    crate::package_model_preview::PreviewBounds { center, radius }
}

fn assert_assembly(actual: &StaticModel) {
    let original = source_model();
    let before = original.source();
    let after = actual.source();
    assert_eq!(after.asset_id, VARIANT_ID);
    assert_eq!(after.primitives.len(), 8);
    assert_eq!(
        after.materials, before.materials,
        "same-source placements reuse complete tables once"
    );
    assert_eq!(after.images, before.images);
    let mut ids = std::collections::BTreeSet::new();
    for (part, key) in ["first", "second"].into_iter().enumerate() {
        for (index, old) in before.primitives.iter().enumerate() {
            let new = &after.primitives[part * 4 + index];
            let mut expected = old.clone();
            expected.id = format!(
                "{}#node=part:{}{}",
                VARIANT_ID,
                key,
                old.id.strip_prefix(SOURCE_ID).unwrap()
            );
            assert!(ids.insert(new.id.clone()), "part identities cannot alias");
            if part == 1 {
                let (sign, y) = if index < 2 {
                    (-1.0, 1.25)
                } else {
                    (1.0, -1.25)
                };
                expected.transform = [
                    [0.0, 0.0, -1.5 * sign, 0.0],
                    [0.0, 0.75, 0.0, 0.0],
                    [0.5, 0.0, 0.0, 0.0],
                    [3.05, 1.0 + y, -2.0, 1.0],
                ];
            }
            assert_eq!(
                new, &expected,
                "only identity and composed placement transform may change"
            );
            if part == 0 {
                assert_eq!(
                    new.transform.map(|v| v.map(f32::to_bits)),
                    old.transform.map(|v| v.map(f32::to_bits)),
                    "identity part preserves signed-zero matrix bits"
                );
            }
            for (old, new) in old.vertices.iter().zip(&new.vertices) {
                assert_eq!(new.uv.map(f32::to_bits), old.uv.map(f32::to_bits));
                assert_eq!(
                    new.position.map(f32::to_bits),
                    old.position.map(f32::to_bits)
                );
                assert_eq!(new.normal.map(f32::to_bits), old.normal.map(f32::to_bits));
            }
        }
    }
    geometry_oracle(actual, None);
    // Two placements record one exact cooked input, without historical metadata.
    let second: [[f32; 4]; 4] = [
        [0.0, 0.0, -2.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.5, 0.0, 0.0, 0.0],
        [3.0, 1.0, -2.0, 1.0],
    ];
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format":"orr_static_model_assemble", "version":1,
        "parts":[
            {"key":"first","source":"input.model.json","placement":orr_model::IDENTITY},
            {"key":"second","source":"input.model.json","placement":second},
        ],
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
                sha256: digest(&original.to_bytes().unwrap())
            },
        ]
    );
}
#[test]
fn actual_static_assembly_widgets_import_install_binding_history_save_reopen_and_cpu_geometry() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let mut import_ui = form(&fixture);
    let before_import = snapshot(&fixture.root);
    click(&mut import_ui, "Import cooked static package");
    wait_import(&mut import_ui);
    let report = import_ui
        .state_mut()
        .2
        .take()
        .expect("worker returned current request");
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.package, PACKAGE);
    assert_eq!(report.kind, ModelKind::Static);
    assert_eq!(
        report.root,
        fixture.temp.path().join("new static assembly package")
    );
    let bytes = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(report.source_sha256, digest(&bytes));
    let license = b"Acceptance fixture: redistribution acknowledged";
    assert_eq!(report.license_sha256, digest(license));
    assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
    assert_assembly(&StaticModel::from_bytes(&bytes).unwrap());
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
        "one cooked asset and one shared license"
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
        "Preview static model imported-static-assembly/model.orrmodel",
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
    assert_assembly(preview.ready().unwrap().loaded.static_model().unwrap());
    let bounds = geometry_oracle(
        preview.ready().unwrap().loaded.static_model().unwrap(),
        None,
    );
    near(preview.ready().unwrap().bounds.center, bounds.center);
    assert!((preview.ready().unwrap().bounds.radius - bounds.radius).abs() < 1e-5);
    click(&mut h, "Use model imported-static-assembly/model.orrmodel");
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
        crate::model_bindings::load_asset_for_kind(&fixture.root, &package, &asset, kind).unwrap();
    let binding =
        crate::model_bindings::Binding::from_asset(package, asset, &loaded, old_binding.transform)
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
        "assembly binding preserves external world placement"
    );
    placed.instance.validate_for(&placed.model).unwrap();
    assert_eq!(placed.model.to_bytes().unwrap(), bytes);
    assert_assembly(&placed.model);
    geometry_oracle(&placed.model, Some(&placed.instance));
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
        if path != Path::new("room.models.json") && path != Path::new("orr.packages.lock.json") {
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

#[test]
fn changed_inspected_assembly_source_worker_rejects_without_publication() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let mut h = form(&fixture);
    // Keep the source valid and geometry identical while changing its exact hash.
    let path = fixture.temp.path().join("author.model.json");
    let mut changed = fs::read(&path).unwrap();
    changed.push(b'\n');
    fs::write(&path, &changed).unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, "Import cooked static package");
    wait_import(&mut h);
    assert!(
        h.state().2.is_none(),
        "stale inspection cannot select a package"
    );
    let status = h.state().0.status().to_ascii_lowercase();
    assert!(
        status.contains("failed") && (status.contains("changed") || status.contains("hash")),
        "{status}"
    );
    assert!(!fixture
        .temp
        .path()
        .join("new static assembly package")
        .exists());
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "checked source rejection leaves no stage/project edits"
    );
    assert_eq!(fs::read(path).unwrap(), changed);
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
}
