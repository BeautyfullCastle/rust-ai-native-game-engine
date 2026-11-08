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
const VARIANT_ID: &str = "variants/prop.retexture.json";
const PIXELS: [u8; 16] = [
    7, 31, 127, 63, 243, 81, 27, 9, 13, 41, 79, 0, 191, 223, 251, 255,
];
const PACKAGE: &str = "imported-static-texture";
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

fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&PIXELS)
            .unwrap();
    }
    bytes
}

fn decoded_png(bytes: &[u8]) -> Image {
    // Decode the input directly, independently of the authoring backend.
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    reader.finish().unwrap();
    pixels.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!((info.width, info.height), (2, 2));
    assert_eq!(pixels, PIXELS);
    Image {
        width: info.width,
        height: info.height,
        rgba8: pixels,
    }
}

fn form(
    fixture: &crate::package_manager::presentation_tests::Fixture,
    png: &[u8],
) -> Harness<'static, ImportState> {
    let before = snapshot(&fixture.root);
    let source = fixture.temp.path().join("author.model.json");
    fs::write(&source, source_model().to_bytes().unwrap()).unwrap();
    let license = fixture.temp.path().join("author-LICENSE.txt");
    fs::write(&license, b"Acceptance fixture: redistribution acknowledged").unwrap();
    let texture = fixture.temp.path().join("skin.png");
    fs::write(&texture, png).unwrap();
    let destination = fixture.temp.path().join("new static texture package");
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
        h.get_by_label("Create static PNG skin")
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
    click(&mut h, "Create static PNG skin");
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
    click(&mut h, "Use material 1");
    text(&mut h, "Static variant PNG file", texture.to_str().unwrap());
    text(&mut h, "Static texture variant asset ID", VARIANT_ID);
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

fn assert_inputs(fixture: &crate::package_manager::presentation_tests::Fixture, png: &[u8]) {
    assert_eq!(
        fs::read(fixture.temp.path().join("author.model.json")).unwrap(),
        source_model().to_bytes().unwrap(),
    );
    assert_eq!(fs::read(fixture.temp.path().join("skin.png")).unwrap(), png);
    assert_eq!(
        fs::read(fixture.temp.path().join("author-LICENSE.txt")).unwrap(),
        b"Acceptance fixture: redistribution acknowledged",
    );
}

fn assert_variant(actual: &StaticModel, png: &[u8]) {
    let original = source_model();
    let before = original.source();
    let after = actual.source();
    assert_eq!(after.asset_id, VARIANT_ID);
    assert_eq!(after.primitives.len(), 4);
    assert_eq!(after.materials.len(), 3);
    assert_eq!(before.materials[0].image, before.materials[1].image);
    assert_eq!(after.images.len(), before.images.len() + 1);
    assert_eq!(
        &after.images[..before.images.len()],
        before.images.as_slice()
    );
    let selected_image = before.images.len() as u32;
    assert_eq!(after.materials[1].image, selected_image);
    assert_eq!(after.images[selected_image as usize], decoded_png(png));
    assert_eq!(after.materials[0], before.materials[0]);
    assert_eq!(after.materials[2], before.materials[2]);
    assert_eq!(
        after.images[after.materials[0].image as usize],
        before.images[0]
    );
    let mut restored = after.clone();
    restored.asset_id = before.asset_id.clone();
    restored.dependencies = before.dependencies.clone();
    restored.materials[1].image = before.materials[1].image;
    restored.images.pop();
    let mut counts = [0; 2];
    for (old, new) in before.primitives.iter().zip(&mut restored.primitives) {
        assert_eq!(
            new.id,
            format!("{}{}", VARIANT_ID, old.id.strip_prefix(SOURCE_ID).unwrap())
        );
        new.id = old.id.clone();
        counts[old.material as usize] += 1;
        for (old, new) in old.vertices.iter().zip(&new.vertices) {
            assert_eq!(new.uv.map(f32::to_bits), old.uv.map(f32::to_bits));
            assert_eq!(
                new.position.map(f32::to_bits),
                old.position.map(f32::to_bits)
            );
            assert_eq!(new.normal.map(f32::to_bits), old.normal.map(f32::to_bits));
        }
    }
    assert_eq!(
        counts,
        [2, 2],
        "both users of selected nonzero and shared-image unselected material"
    );
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(before).unwrap(),
        "only selected material image, appended pixels and identity/provenance may change",
    );
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format":"orr_static_material_texture", "version":1,
        "source":"input.model.json", "texture":"skin.png", "material":1,
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
            Dependency {
                uri: "skin.png".into(),
                sha256: digest(png)
            },
        ]
    );
}

#[test]
fn actual_static_texture_widgets_import_install_binding_history_save_reopen_and_cpu_materials() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let png = png_bytes();
    let mut import_ui = form(&fixture, &png);
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
        fixture.temp.path().join("new static texture package")
    );
    let bytes = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(report.source_sha256, digest(&bytes));
    let license = b"Acceptance fixture: redistribution acknowledged";
    assert_eq!(report.license_sha256, digest(license));
    assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
    assert_variant(&StaticModel::from_bytes(&bytes).unwrap(), &png);
    assert_inputs(&fixture, &png);
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
        "Preview static model imported-static-texture/model.orrmodel",
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
        &png,
    );
    click(&mut h, "Use model imported-static-texture/model.orrmodel");
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
        "binding texture edits preserve world placement"
    );
    placed.instance.validate_for(&placed.model).unwrap();
    assert_eq!(placed.model.to_bytes().unwrap(), bytes);
    assert_variant(&placed.model, &png);
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
    assert_inputs(&fixture, &png);
}

#[test]
fn invalid_static_texture_png_worker_rejects_without_publishing_or_editing_project() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let malformed = b"not a PNG image";
    let mut h = form(&fixture, malformed);
    let before = snapshot(fixture.temp.path());
    click(&mut h, "Import cooked static package");
    wait_import(&mut h);
    assert!(
        h.state().2.is_none(),
        "failed PNG import cannot select a package"
    );
    let status = h.state().0.status().to_ascii_lowercase();
    assert!(
        status.contains("failed") && status.contains("png"),
        "{status}"
    );
    assert!(!fixture
        .temp
        .path()
        .join("new static texture package")
        .exists());
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "failed import leaves no package or staging tree"
    );
    assert_inputs(&fixture, malformed);
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
}
