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

const VARIANT_ID: &str = "props/key.cylinder.json";
const PACKAGE: &str = "created-cylinder";
const IMPORT: &str = "Create capped cylinder package";
const RADIUS: f64 = 1.25;
const HEIGHT: f64 = 3.5;
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
    segments: u32,
) -> Harness<'static, ImportState> {
    let license = fixture.temp.path().join("cylinder-LICENSE.txt");
    fs::write(&license, b"Acceptance fixture: redistribution acknowledged").unwrap();
    let destination = fixture.temp.path().join("new cylinder package");
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
    click(&mut h, "Create cylinder package");
    assert!(h.query_by_label("Raw GLB file").is_none());
    assert!(h
        .query_by_label("Cooked static model file (.model.json)")
        .is_none());
    assert!(h.query_by_label("Inspect static source").is_none());
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "license and complete package metadata remain mandatory"
    );
    for (label, value) in [
        ("License file", license.to_str().unwrap()),
        (
            "Package destination directory",
            destination.to_str().unwrap(),
        ),
        ("Package name", PACKAGE),
        ("Package version", "1.0.0"),
        ("Cylinder radius", "1.25"),
        ("Cylinder height", "3.5"),
        ("Cylinder asset ID", VARIANT_ID),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Cylinder segments");
    click(&mut h, &format!("{segments} cylinder segments"));
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "parameters cannot grant distribution consent"
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
fn assert_license(fixture: &crate::package_manager::presentation_tests::Fixture) {
    assert_eq!(
        fs::read(fixture.temp.path().join("cylinder-LICENSE.txt")).unwrap(),
        b"Acceptance fixture: redistribution acknowledged"
    );
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.000002,
        "actual {actual} != expected {expected}"
    );
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

// Analytic geometry/topology oracle, independent of the generator and its table.
fn assert_cylinder(model: &StaticModel, segments: u32) {
    let source = model.source();
    let n = segments as usize;
    assert_eq!(source.format, "orr_static_model");
    assert_eq!(source.version, 1);
    assert_eq!(source.asset_id, VARIANT_ID);
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(
        source.images,
        vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4]
        }]
    );
    for material in &source.materials {
        assert_eq!(material.base_color, [1.0; 4]);
        assert_eq!(material.image, 0);
        assert_eq!(
            (material.linear_filter, material.wrap_s, material.wrap_t),
            (false, Wrap::Clamp, Wrap::Clamp)
        );
    }
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    let mut triangle_count = 0;
    for (slot, primitive) in source.primitives.iter().enumerate() {
        assert_eq!(
            primitive.id,
            format!("{VARIANT_ID}#node=0/mesh=0/primitive={slot}")
        );
        assert_eq!(primitive.material, slot as u32);
        assert_eq!(primitive.transform, orr_model::IDENTITY);
        assert_eq!(
            primitive.vertices.len(),
            if slot == 0 { 2 * (n + 1) } else { n + 1 }
        );
        assert_eq!(
            primitive.indices.len(),
            if slot == 0 { 6 * n } else { 3 * n }
        );
        for vertex in &primitive.vertices {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(vertex.position[axis]);
                maximum[axis] = maximum[axis].max(vertex.position[axis]);
            }
            assert!(vertex.uv.iter().all(|v| (0.0..=1.0).contains(v)));
            close(
                dot(vertex.normal.map(f64::from), vertex.normal.map(f64::from)),
                1.0,
            );
        }
        for triangle in primitive.indices.chunks_exact(3) {
            let p = [triangle[0], triangle[1], triangle[2]]
                .map(|index| primitive.vertices[index as usize].position.map(f64::from));
            let face = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            let center: [f64; 3] =
                std::array::from_fn(|axis| (p[0][axis] + p[1][axis] + p[2][axis]) / 3.0);
            let outward = match slot {
                0 => [center[0], 0.0, center[2]],
                1 => [0.0, 1.0, 0.0],
                _ => [0.0, -1.0, 0.0],
            };
            assert!(
                dot(face, outward) > 0.0,
                "nondegenerate outward triangle in slot{slot}"
            );
            for index in triangle {
                assert!(
                    dot(
                        face,
                        primitive.vertices[*index as usize].normal.map(f64::from)
                    ) > 0.0
                );
            }
            triangle_count += 1;
        }
    }
    assert_eq!(triangle_count, 4 * n);
    assert_eq!(minimum, [-1.25, -1.75, -1.25]);
    assert_eq!(maximum, [1.25, 1.75, 1.25]);
    let side = &source.primitives[0];
    for i in 0..=n {
        let angle = std::f64::consts::TAU * i as f64 / n as f64;
        for (offset, y, v) in [(0, -1.75, 1.0), (1, 1.75, 0.0)] {
            let vertex = &side.vertices[2 * i + offset];
            close(f64::from(vertex.position[0]), RADIUS * angle.cos());
            close(f64::from(vertex.position[2]), RADIUS * angle.sin());
            assert_eq!(vertex.position[1], y);
            close(f64::from(vertex.normal[0]), angle.cos());
            close(f64::from(vertex.normal[2]), angle.sin());
            assert_eq!(vertex.normal[1], 0.0);
            assert_eq!(vertex.uv, [i as f32 / n as f32, v]);
        }
    }
    for offset in [0, 1] {
        assert_eq!(
            side.vertices[offset].position.map(f32::to_bits),
            side.vertices[2 * n + offset].position.map(f32::to_bits)
        );
        assert_eq!(
            side.vertices[offset].normal.map(f32::to_bits),
            side.vertices[2 * n + offset].normal.map(f32::to_bits)
        );
        assert_eq!(
            (
                side.vertices[offset].uv[0],
                side.vertices[2 * n + offset].uv[0]
            ),
            (0.0, 1.0)
        );
    }
    for (slot, y, normal) in [(1, 1.75, [0.0, 1.0, 0.0]), (2, -1.75, [0.0, -1.0, 0.0])] {
        let cap = &source.primitives[slot];
        assert_eq!(cap.vertices[0].position, [0.0, y, 0.0]);
        assert_eq!(cap.vertices[0].uv, [0.5, 0.5]);
        for (i, vertex) in cap.vertices.iter().enumerate() {
            assert_eq!(vertex.normal, normal);
            assert_eq!(vertex.position[1], y);
            close(
                f64::from(vertex.uv[0]),
                0.5 + 0.5 * f64::from(vertex.position[0]) / RADIUS,
            );
            close(
                f64::from(vertex.uv[1]),
                0.5 + 0.5 * f64::from(vertex.position[2]) / RADIUS,
            );
            if i > 0 {
                close(
                    f64::from(vertex.position[0]).hypot(f64::from(vertex.position[2])),
                    RADIUS,
                );
                assert_eq!(
                    vertex.position.map(f32::to_bits),
                    side.vertices[2 * (i - 1) + usize::from(slot == 1)]
                        .position
                        .map(f32::to_bits)
                );
            }
        }
    }
    let recipe=serde_json::to_vec(&serde_json::json!({
        "format":"orr_static_model_cylinder","version":1,"radius":RADIUS,"height":HEIGHT,"segments":segments,
    })).unwrap();
    assert_eq!(
        source.dependencies,
        vec![Dependency {
            uri: "$source".into(),
            sha256: digest(&recipe)
        }]
    );
}
#[test]
fn actual_cylinder_widgets_two_segment_choices_install_key_history_save_reopen_and_cpu_geometry() {
    for segments in [8, 32] {
        let fixture = crate::package_manager::presentation_tests::fixture(false, false);
        let mut import_ui = form(&fixture, segments);
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
        assert_eq!(
            report.root,
            fixture.temp.path().join("new cylinder package")
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(report.root.join("orr.package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["capabilities"], serde_json::json!(["models"]));
        let bytes = fs::read(report.root.join(&report.asset)).unwrap();
        assert_eq!(report.source_sha256, digest(&bytes));
        let license = b"Acceptance fixture: redistribution acknowledged";
        assert_eq!(report.license_sha256, digest(license));
        assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
        assert_cylinder(&StaticModel::from_bytes(&bytes).unwrap(), segments);
        assert_license(&fixture);
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
            "Preview static model created-cylinder/model.orrmodel",
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
        assert_cylinder(
            preview.ready().unwrap().loaded.static_model().unwrap(),
            segments,
        );
        click(&mut h, "Use model created-cylinder/model.orrmodel");
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
            "source-less cylinder keeps the KEY placement separate"
        );
        placed.instance.validate_for(&placed.model).unwrap();
        assert_eq!(placed.model.to_bytes().unwrap(), bytes);
        assert_cylinder(&placed.model, segments);
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
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
        assert_license(&fixture);
    }
}

#[test]
fn invalid_cylinder_dimensions_and_existing_destination_never_publish_or_overwrite() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let mut h = form(&fixture, 16);
    let before = snapshot(fixture.temp.path());
    text(&mut h, "Cylinder radius", "0");
    assert!(h.get_by_label(IMPORT).accesskit_node().is_disabled());
    assert!(h.state().2.is_none());
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "invalid draft cannot publish"
    );
    text(&mut h, "Cylinder radius", "1.25");
    assert_eq!(
        h.get_by_label(RIGHTS).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    click(&mut h, RIGHTS);
    let destination = fixture.temp.path().join("new cylinder package");
    fs::create_dir(&destination).unwrap();
    fs::write(
        destination.join("sentinel.txt"),
        b"existing caller content; never overwrite",
    )
    .unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, IMPORT);
    wait_import(&mut h);
    assert!(
        h.state().2.is_none(),
        "failed creation cannot select an existing destination"
    );
    let status = h.state().0.status().to_ascii_lowercase();
    assert!(
        status.contains("exist") || status.contains("new destination"),
        "{status}"
    );
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "rejection cannot replace files or leave stages"
    );
    assert!(!destination.join("model.orrmodel").exists());
    assert_license(&fixture);
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
}
