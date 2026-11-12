//! CPU acceptance: real import/package widgets, then validated binding transactions.
//! Assignment/history/save use Bindings' production API, not simulated UI clicks.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_model::{animation::*, Dependency, Image, Material, StaticModel, Vertex, Wrap, IDENTITY};
use sha2::{Digest, Sha256};
use std::{
    fs,
    time::{Duration, Instant},
};

const SOURCE_ID: &str = "actors/original.glb";
const VARIANT_ID: &str = "variants/actor.pose.json";
const PACKAGE: &str = "imported-pose-prop";
const IMPORT: &str = "Import baked static pose package";
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

// Bounded weighted-and-rigid source copied from the existing pose producer's
// literal integration fixture. Expected samples below never call sample/deform.
fn source_model() -> AnimatedModel {
    let node = |name: &str, children, translation, scale| AnimatedNode {
        name: name.into(),
        children,
        rest: Trs {
            translation,
            scale,
            ..Trs::default()
        },
    };
    let vertices: Vec<_> = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
        .into_iter()
        .enumerate()
        .map(|(i, position)| SkinnedVertex {
            vertex: Vertex {
                position,
                normal: [0.6, 0.8, 0.],
                uv: if i == 0 { [-0., 0.] } else { [i as f32, -0.] },
            },
            joints: [0, 1, 0, 0],
            weights: [0.25, 0.75, 0., 0.],
        })
        .collect();
    let mut rigid = vertices.clone();
    for v in &mut rigid {
        v.joints = [0; 4];
        v.weights = [1., 0., 0., 0.];
    }
    let mut ib0 = IDENTITY;
    ib0[3][0] = -0.5;
    let mut ib1 = IDENTITY;
    ib1[3] = [0., -1., 1., 1.];
    AnimatedModel::new(AnimatedSource {
        format: "orr_animated_model".into(),
        version: 1,
        asset_id: "actors/original.glb".into(),
        dependencies: vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(b"original"),
            },
            Dependency {
                uri: "historical/missing.png".into(),
                sha256: digest(b"missing"),
            },
        ],
        images: vec![
            Image {
                width: 1,
                height: 1,
                rgba8: vec![19, 37, 71, 103],
            },
            Image {
                width: 1,
                height: 1,
                rgba8: vec![2, 3, 5, 7],
            },
        ],
        materials: vec![
            Material {
                base_color: [-0., 0.5, 0.75, 0.25],
                image: 0,
                linear_filter: true,
                wrap_s: Wrap::Mirror,
                wrap_t: Wrap::Repeat,
            },
            Material {
                base_color: [1., 0.25, 0., 1.],
                image: 1,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Mirror,
            },
        ],
        nodes: vec![
            node("ancestor", vec![1, 2, 3, 4], [10., 20., 30.], [2., 3., 4.]),
            node("joint-a", vec![], [1., 2., 3.], [1., 2., 1.]),
            node("joint-b", vec![], [-1., 1., 0.], [2., 1., 0.5]),
            node("skinned-mesh", vec![], [100., 200., 300.], [3., 5., 7.]),
            node("rigid-mesh", vec![], [7., 8., 9.], [5., 2., 1.]),
        ],
        skins: vec![Skin {
            name: "skin".into(),
            joints: vec![1, 2],
            inverse_bind_matrices: vec![ib0, ib1],
        }],
        primitives: vec![
            AnimatedPrimitive {
                id: "actors/original.glb#node=3/mesh=0/primitive=0".into(),
                node: 3,
                skin: Some(0),
                vertices,
                indices: vec![0, 1, 2],
                material: 0,
            },
            AnimatedPrimitive {
                id: "actors/original.glb#node=4/mesh=1/primitive=0".into(),
                node: 4,
                skin: None,
                vertices: rigid,
                indices: vec![2, 1, 0],
                material: 1,
            },
        ],
        clips: vec![
            AnimationClip {
                name: "move".into(),
                channels: vec![AnimationChannel {
                    node: 0,
                    interpolation: Interpolation::Linear,
                    times: vec![0., 1.],
                    values: ChannelValues::Translation(vec![[10., 20., 30.], [12., 20., 30.]]),
                }],
            },
            AnimationClip {
                name: "other".into(),
                channels: vec![AnimationChannel {
                    node: 0,
                    interpolation: Interpolation::Step,
                    times: vec![0., 1.],
                    values: ChannelValues::Translation(vec![[50., 20., 30.], [60., 20., 30.]]),
                }],
            },
        ],
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
    clip: u32,
    time: f64,
) -> Harness<'static, ImportState> {
    let before = snapshot(&fixture.root);
    let source = fixture.temp.path().join("author.animated.json");
    fs::write(&source, source_model().to_bytes().unwrap()).unwrap();
    let license = fixture.temp.path().join("author-LICENSE.txt");
    fs::write(&license, b"Acceptance fixture: redistribution acknowledged").unwrap();
    let destination = fixture.temp.path().join("new static pose package");
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
    click(&mut h, "Cooked animated model");
    click(&mut h, "Animated model");
    assert_eq!(
        h.get_by_label("Bake static prop from animation")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
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
        ("Package name", PACKAGE),
        ("Package version", "1.0.0"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Bake static prop from animation");
    assert!(h.get_by_label(IMPORT).accesskit_node().is_disabled());
    let inspection_files = snapshot(fixture.temp.path());
    click(&mut h, "Inspect animated source");
    wait_import(&mut h);
    assert_eq!(
        snapshot(fixture.temp.path()),
        inspection_files,
        "inspection is read-only"
    );
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "inspection cannot auto-select clip0"
    );
    click(&mut h, &format!("Use clip {clip}"));
    text(&mut h, "Pose sample time (seconds)", &time.to_string());
    text(&mut h, "Static pose asset ID", VARIANT_ID);
    assert!(
        h.get_by_label(IMPORT).accesskit_node().is_disabled(),
        "clip selection cannot grant rights"
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
        fs::read(fixture.temp.path().join("author.animated.json")).unwrap(),
        source_model().to_bytes().unwrap()
    );
    assert_eq!(
        fs::read(fixture.temp.path().join("author-LICENSE.txt")).unwrap(),
        b"Acceptance fixture: redistribution acknowledged"
    );
}
fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 0.0001, "actual {a} != expected {b}");
    }
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|value| value * value).sum::<f32>().sqrt();
    v.map(|value| value / length)
}
fn assert_baked(model: &StaticModel, clip: u32, time: f64) {
    let source = source_model();
    let before = source.source();
    let after = model.source();
    assert_eq!(after.format, "orr_static_model");
    assert_eq!(after.version, 1);
    assert_eq!(after.asset_id, VARIANT_ID);
    assert_eq!(after.images, before.images);
    assert_eq!(
        serde_json::to_vec(&after.materials).unwrap(),
        serde_json::to_vec(&before.materials).unwrap()
    );
    assert_eq!(after.primitives.len(), 2);
    // Linear half-time and the nonzero Step clip's exact final key.
    let root_x = if clip == 0 {
        assert_eq!(time, 0.5);
        11.0
    } else {
        assert_eq!((clip, time), (1, 1.0));
        60.0
    };
    for (index, (old, new)) in before.primitives.iter().zip(&after.primitives).enumerate() {
        assert_eq!(new.id, old.id.replacen(SOURCE_ID, VARIANT_ID, 1));
        assert_eq!(
            new.transform, IDENTITY,
            "pose is model-space geometry; entity placement is not baked"
        );
        assert_eq!(new.indices, old.indices);
        assert_eq!(new.material, old.material);
        assert_eq!(new.vertices.len(), old.vertices.len());
        for (old, new) in old.vertices.iter().zip(&new.vertices) {
            let [x, y, z] = old.vertex.position;
            let expected = if index == 0 {
                // Two joint globals, inverse binds, then .25/.75 weights.
                // The skinned mesh node's large TRS must not be applied again.
                [3.5 * x + root_x - 1.25, 3.75 * y + 21.5, 2.5 * z + 34.5]
            } else {
                [10.0 * x + root_x + 14.0, 6.0 * y + 44.0, 4.0 * z + 66.0]
            };
            near(new.position, expected);
            near(
                new.normal,
                unit(if index == 0 {
                    [0.6 / 3.5, 0.8 / 3.75, 0.0]
                } else {
                    [0.6 / 10.0, 0.8 / 6.0, 0.0]
                }),
            );
            assert_eq!(new.uv.map(f32::to_bits), old.vertex.uv.map(f32::to_bits));
        }
        assert_ne!(new.vertices[0].position, old.vertices[0].vertex.position);
        assert_ne!(new.vertices[0].normal, old.vertices[0].vertex.normal);
    }
    let recipe=serde_json::to_vec(&serde_json::json!({
        "format":"orr_animation_pose_bake","version":1,"source":"input.animated.json","clip":clip,"time":time,
    })).unwrap();
    assert_eq!(
        after.dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(&recipe)
            },
            Dependency {
                uri: "input.animated.json".into(),
                sha256: digest(&source.to_bytes().unwrap())
            },
        ]
    );
    let json = serde_json::to_value(after).unwrap();
    for field in ["nodes", "skins", "clips"] {
        assert!(
            json.get(field).is_none(),
            "baked static source cannot retain {field}"
        );
    }
}
#[test]
fn actual_pose_bake_widgets_install_static_prop_history_save_reopen_and_literal_cpu_geometry() {
    for (clip, time) in [(0, 0.5), (1, 1.0)] {
        let fixture = crate::package_manager::presentation_tests::fixture(false, false);
        let mut import_ui = form(&fixture, clip, time);
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
            fixture.temp.path().join("new static pose package")
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(report.root.join("orr.package.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["capabilities"],
            serde_json::json!(["models"]),
            "animated source must publish models-only output"
        );
        let bytes = fs::read(report.root.join(&report.asset)).unwrap();
        assert_eq!(report.source_sha256, digest(&bytes));
        let license = b"Acceptance fixture: redistribution acknowledged";
        assert_eq!(report.license_sha256, digest(license));
        assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
        assert_baked(&StaticModel::from_bytes(&bytes).unwrap(), clip, time);
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
            "Preview static model imported-pose-prop/model.orrmodel",
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
        assert_baked(
            preview.ready().unwrap().loaded.static_model().unwrap(),
            clip,
            time,
        );
        assert!(h
            .query_by_label("Use animated model imported-pose-prop/model.orrmodel")
            .is_none());
        click(&mut h, "Use model imported-pose-prop/model.orrmodel");
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
        assert!(
            loaded.animated_model().is_none(),
            "baked package cannot be loaded as animated"
        );
        assert!(crate::model_bindings::load_asset_for_kind(
            &fixture.root,
            &package,
            &asset,
            ModelKind::Animated
        )
        .is_err());
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
        assert_eq!(binding.kind, ModelKind::Static);
        assert!(binding.animation.is_none());
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
            "baked prop retains separate ordinary Room placement"
        );
        placed.instance.validate_for(&placed.model).unwrap();
        assert_eq!(placed.model.to_bytes().unwrap(), bytes);
        assert_baked(&placed.model, clip, time);
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
            fs::read(fixture.temp.path().join("author.animated.json")).unwrap(),
            source_model().to_bytes().unwrap()
        );
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
        assert_inputs(&fixture);
    }
}

#[test]
fn changed_inspected_pose_source_rejects_without_publishing_or_editing_room() {
    let fixture = crate::package_manager::presentation_tests::fixture(false, false);
    let mut h = form(&fixture, 1, 1.0);
    let source = fixture.temp.path().join("author.animated.json");
    let mut changed = fs::read(&source).unwrap();
    changed.push(b'\n');
    fs::write(&source, &changed).unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, IMPORT);
    wait_import(&mut h);
    assert!(h.state().2.is_none());
    let status = h.state().0.status().to_ascii_lowercase();
    assert!(
        status.contains("failed") && (status.contains("changed") || status.contains("hash")),
        "{status}"
    );
    assert!(!fixture.temp.path().join("new static pose package").exists());
    assert_eq!(snapshot(fixture.temp.path()), before);
    assert_eq!(fs::read(source).unwrap(), changed);
    assert!(!package_manager::inspect(&fixture.root)
        .unwrap()
        .packages
        .contains_key(PACKAGE));
}
