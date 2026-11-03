//! Actual public dependency-update Panel widgets and worker, followed by installed Room
//! model consumer. CPU-only: no GPU, window or rendered-appearance claim.
#![cfg(feature = "package-manager")]
#![allow(clippy::disallowed_types)]
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::package_manager_panel::Panel;
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::Project;
use orr_reflect::Scene;
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const BASELINE: &str = "baseline-model";
const A: &str = "dependency-root";
const B: &str = "dependency-middle";
const C: &str = "dependency-model";
const D: &str = "unused-candidate";
const ASSET: &str = "foreground.glb";
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn source(
    root: &Path,
    name: &str,
    version: &str,
    dependency: Option<&str>,
    model: bool,
) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    let mut dependencies = BTreeMap::new();
    if let Some(name) = dependency {
        dependencies.insert(name, "1.0.0");
    }
    let names = if model {
        let original =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for name in [ASSET, "LICENSE.txt"] {
            fs::copy(original.join(name), path.join(name)).unwrap();
        }
        vec![ASSET, "LICENSE.txt"]
    } else {
        fs::write(
            path.join("content.txt"),
            format!("Data from {name} version {version}\n"),
        )
        .unwrap();
        fs::write(
            path.join("LICENSE.txt"),
            "Synthetic integration-test data; fixture author grants use under MIT OR Apache-2.0.\n",
        )
        .unwrap();
        vec!["content.txt", "LICENSE.txt"]
    };
    fs::write(path.join("orr.package.json"),serde_json::to_vec(&serde_json::json!({
        "schema":1,"name":name,"version":version,"engine":"*",
        "capabilities":if model {vec!["models"]}else{vec![]},"dependencies":dependencies,"files":names
    })).unwrap()).unwrap();
    path
}
fn room(root: &Path, sources: &Path) {
    fs::create_dir(root).unwrap();
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap()).unwrap();
    let baseline = source(sources, BASELINE, "1.0.0", None, true);
    Project::open_for_install(root, room_project::compiled_runtime().engine_version)
        .unwrap()
        .install(&[baseline])
        .unwrap();
    let loaded = model_bindings::load_asset(root, BASELINE, ASSET).unwrap();
    let binding = Binding::from_asset(
        BASELINE.into(),
        ASSET.into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let player = scene
        .frame()
        .entities()
        .find(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap();
    let guid = scene.index().guid(player).unwrap().to_string();
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: BTreeMap::from([(guid, binding)]),
        })
        .unwrap(),
    )
    .unwrap();
    PreparedProject::open(root).expect("baseline model binding admits the Room project");
}
fn click(h: &mut Harness<'_, Panel>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(8);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    h.run_steps(5);
}
fn wait(h: &mut Harness<'_, Panel>, prefix: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run_steps(2);
        if !h.state().busy() {
            assert!(
                h.state().status.starts_with(prefix),
                "expected {prefix}: {}",
                h.state().status
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "worker timeout: {}",
            h.state().status
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn actual_dependency_update_preserves_baseline_and_room_reopens_transitive_model_without_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Room project");
    let sources = temp.path().join("package sources");
    fs::create_dir(&sources).unwrap();
    room(&root, &sources);
    let old_sources = sources.join("old release");
    let new_sources = sources.join("new release");
    fs::create_dir(&old_sources).unwrap();
    fs::create_dir(&new_sources).unwrap();
    let old_a = source(&old_sources, A, "1.0.0", None, false);
    Project::open_for_install(&root, room_project::compiled_runtime().engine_version)
        .unwrap()
        .install(&[old_a])
        .unwrap();
    let initial_lock = Project::open(&root, room_project::compiled_runtime())
        .unwrap()
        .verify()
        .unwrap();
    assert_eq!(initial_lock.direct[A], "1.0.0");
    assert_eq!(initial_lock.packages.len(), 2);
    let a = source(&new_sources, A, "2.0.0", Some(B), false);
    let b = source(&new_sources, B, "1.0.0", Some(C), false);
    let c = source(&new_sources, C, "1.0.0", None, true);
    let d = source(&new_sources, D, "1.0.0", None, false);
    let original_asset = fs::read(c.join(ASSET)).unwrap();
    let original_license = fs::read(c.join("LICENSE.txt")).unwrap();
    let source_files = files(&sources);
    let project_before = files(&root);
    let frame_before = PreparedProject::open(&root)
        .unwrap()
        .scene()
        .frame()
        .to_bytes();
    let root_for_ui = root.clone();
    let panel = Panel::default();
    let mut h = Harness::builder().with_size([1500., 1500.]).build_ui_state(
        move |ui, panel: &mut Panel| panel.show(ui.ctx(), Some(&root_for_ui), None),
        panel,
    );
    h.state_mut().open = true;
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Local content package directory")
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Local content package directory")
        .focus();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Local content package directory")
        .type_text(a.to_str().unwrap());
    h.run_steps(3);
    let candidate_text = format!("{}\n{}\n{}", b.display(), c.display(), d.display());
    h.get_by_role_and_label(
        Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    )
    .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(
        Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    )
    .focus();
    h.run_steps(2);
    h.get_by_role_and_label(
        Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    )
    .type_text(&candidate_text);
    h.run_steps(3);
    assert_eq!(h.state().source, a.to_str().unwrap());
    assert_eq!(
        h.get_by_role_and_label(
            Role::MultilineTextInput,
            "Dependency candidate directories (one per line)"
        )
        .value()
        .as_deref(),
        Some(candidate_text.as_str())
    );
    click(&mut h, "Preview package update with dependencies");
    wait(&mut h, "Dependency update preview ready;");
    assert_eq!(
        files(&root),
        project_before,
        "Preview must not create lock/cache changes"
    );
    assert_eq!(
        files(&sources),
        source_files,
        "Preview cannot modify candidates"
    );
    assert!(h
        .query_by_label(&format!("Unused update candidate: {D}"))
        .is_some());
    click(&mut h, "Update package with dependencies");
    wait(&mut h, "Package dependencies updated to current project.");
    assert_eq!(files(&sources), source_files);
    let project = Project::open(&root, room_project::compiled_runtime()).unwrap();
    let lock = project.verify().unwrap();
    assert_eq!(
        lock.direct.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![BASELINE, A]
    );
    assert_eq!(lock.packages.len(), 4);
    assert_eq!(lock.direct[A], "2.0.0");
    assert_eq!(lock.packages[A].manifest.version, "2.0.0");
    assert_ne!(lock.packages[A].digest, initial_lock.packages[A].digest);
    assert_eq!(
        project.read_asset(A, "content.txt").unwrap(),
        b"Data from dependency-root version 2.0.0\n"
    );
    assert_eq!(lock.packages[BASELINE], initial_lock.packages[BASELINE]);
    assert_eq!(lock.direct[BASELINE], initial_lock.direct[BASELINE]);
    for name in [A, B, C] {
        assert!(lock.packages.contains_key(name));
    }
    assert!(!lock.direct.contains_key(B));
    assert!(!lock.direct.contains_key(C));
    assert!(!lock.packages.contains_key(D));
    assert_eq!(
        fs::read_dir(root.join(".orr/packages/objects"))
            .unwrap()
            .count(),
        5,
        "baseline, old retained A and new A/B/C only; unused D must not be cached"
    );
    assert_eq!(project.read_asset(C, ASSET).unwrap(), original_asset);
    assert_eq!(
        project.read_asset(C, "LICENSE.txt").unwrap(),
        original_license
    );
    for (path, bytes) in &project_before {
        if path != Path::new("orr.packages.lock.json") {
            assert_eq!(fs::read(root.join(path)).unwrap(), *bytes);
        }
    }
    assert_eq!(
        PreparedProject::open(&root)
            .unwrap()
            .scene()
            .frame()
            .to_bytes(),
        frame_before
    );
    drop(project);
    drop(h);
    fs::rename(&sources, temp.path().join("hidden package sources")).unwrap();
    assert!(!sources.exists());
    // Consume transitive C through the normal compiled-runtime package and model
    // loaders, not via its original source directory or a direct lock entry.
    let project = Project::open(&root, room_project::compiled_runtime()).unwrap();
    project.verify().unwrap();
    assert_eq!(project.read_asset(C, ASSET).unwrap(), original_asset);
    let loaded = model_bindings::load_asset(&root, C, ASSET).unwrap();
    assert!(!loaded
        .static_model()
        .unwrap()
        .source()
        .primitives
        .is_empty());
    let binding =
        Binding::from_asset(C.into(), ASSET.into(), &loaded, LocalTransform::default()).unwrap();
    let text = fs::read_to_string(root.join("room.scene.yaml")).unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let player = scene
        .frame()
        .entities()
        .find(|e| {
            scene
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| a.kind == PLAYER)
        })
        .unwrap();
    let guid = scene.index().guid(player).unwrap().to_string();
    // Test-authored persisted binding through the public consumer API; this is
    // not an editor model-binding Save-button test.
    let document = Document {
        version: 2,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings: BTreeMap::from([(guid, binding.clone())]),
    };
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    let rebound = model_bindings::load_binding(&root, &binding).unwrap();
    assert_eq!(rebound.package_digest(), loaded.package_digest());
    assert_eq!(rebound.source_hash(), loaded.source_hash());
    assert_eq!(
        rebound.static_model().unwrap().source(),
        loaded.static_model().unwrap().source()
    );
    let prepared = PreparedProject::open(&root).unwrap();
    assert_eq!(prepared.models().document, document);
    assert_eq!(prepared.scene().frame().to_bytes(), frame_before);
    let installed = &prepared.models().assets[&(C.into(), ASSET.into())];
    assert_eq!(installed.source_hash(), loaded.source_hash());
    assert_eq!(installed.package_digest(), lock.packages[C].digest);
    assert_eq!(
        installed.static_model().unwrap().source(),
        loaded.static_model().unwrap().source()
    );
    assert_eq!(
        files(&temp.path().join("hidden package sources")),
        source_files
    );
}
