use super::*;
use crate::{model_panel::ModelPanel, package_manager, Editor, EditorApp, HostSpec, Target};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_reflect::Guid;
use orr_sample::room_project::{PreparedProject, PreparedScene};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

const PACKAGE: &str = "usage-models";
const ASSET: &str = "one.glb";

struct Fixture {
    temporary: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary =
            tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let root = temporary.path().join("room");
        let source = temporary.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&source).unwrap();
        for asset in [ASSET, "two.glb"] {
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../assets/imported_scene_demo/foreground.glb"),
                source.join(asset),
            )
            .unwrap();
        }
        fs::write(source.join("LICENSE.txt"), "Owned fixture").unwrap();
        fs::write(source.join("orr.package.json"), serde_json::to_vec(&serde_json::json!({
            "schema": 1, "name": PACKAGE, "version": "1.0.0", "engine": "*",
            "capabilities": ["models"], "dependencies": {}, "files": [ASSET, "two.glb", "LICENSE.txt"]
        })).unwrap()).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec(&serde_json::json!({
            "schema": 2, "engine": "*", "entry": {
                "game": "room-escape-v1", "scene": "room.scene.yaml", "models": "room.models.json"
            }
        })).unwrap()).unwrap();
        let text = include_str!("../../../tests/fixtures/room_initial.scene.yaml");
        fs::write(root.join("room.scene.yaml"), text).unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(std::slice::from_ref(&source))
            .unwrap();
        let scene = PreparedScene::parse(text).unwrap();
        let bindings = scene
            .frame()
            .entities()
            .take(2)
            .zip([ASSET, "two.glb"])
            .map(|(entity, asset)| {
                let loaded = model_bindings::load_asset(&root, PACKAGE, asset).unwrap();
                let binding = Binding::from_asset(
                    PACKAGE.into(),
                    asset.into(),
                    &loaded,
                    LocalTransform::default(),
                )
                .unwrap();
                (scene.index().guid(entity).unwrap().to_string(), binding)
            })
            .collect();
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&Document {
                version: 2,
                scene: "room.scene.yaml".into(),
                project: ".".into(),
                bindings,
            })
            .unwrap(),
        )
        .unwrap();
        Self {
            temporary,
            root,
            source,
        }
    }

    fn panel(&self) -> Panel {
        let mut panel = Panel {
            open: true,
            root: Some(self.root.clone()),
            project_generation: 4,
            verified: Some(package_manager::inspect(&self.root).unwrap()),
            ..Panel::default()
        };
        panel.usage.select(PACKAGE.into());
        panel
    }

    fn identity(&self) -> Identity {
        self.panel().usage_identity().unwrap()
    }

    fn request(&self) -> authoring::Request {
        let identity = self.identity();
        authoring::Request {
            project: identity.root,
            package: identity.package,
            digest: identity.digest,
        }
    }

    fn install(&self, name: &str, capabilities: &[&str]) -> Lock {
        let source = self.temporary.path().join(name);
        fs::create_dir(&source).unwrap();
        fs::write(source.join("LICENSE.txt"), "Owned unused fixture").unwrap();
        fs::write(
            source.join("orr.package.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": 1, "name": name, "version": "1.0.0", "engine": "*",
                "capabilities": capabilities, "dependencies": {}, "files": ["LICENSE.txt"]
            }))
            .unwrap(),
        )
        .unwrap();
        Project::open_for_install(&self.root, Runtime::content_only().engine_version)
            .unwrap()
            .install(&[source])
            .unwrap()
    }

    fn reject_unchanged(&self, identity: &Identity) {
        let before = snapshot(self.temporary.path());
        assert!(inspect(identity).is_err());
        assert_eq!(snapshot(self.temporary.path()), before);
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

fn wait(panel: &mut Panel) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while panel.busy() {
        panel.poll();
        assert!(Instant::now() < deadline, "{}", panel.status);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn completed(panel: &mut Panel, report: UsageReport) {
    let identity = panel.usage_identity().unwrap();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion::UsageInspected {
            identity,
            result: Ok(report),
        })
        .unwrap();
    panel.pending = Some(receiver);
}

#[test]
fn shared_query_output_is_exact_deterministic_and_read_only_without_sources() {
    let fixture = Fixture::new();
    let identity = fixture.identity();
    let expected = authoring::inspect_usage(&fixture.request()).unwrap();
    fs::rename(
        &fixture.source,
        fixture.temporary.path().join("hidden source"),
    )
    .unwrap();
    let before = snapshot(fixture.temporary.path());
    let report = inspect(&identity).unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(report.bindings.len(), 2);
    assert!(report.bindings[0].guid < report.bindings[1].guid);
    assert_ne!(report.bindings[0].asset, report.bindings[1].asset);
    assert_eq!(snapshot(fixture.temporary.path()), before);
}

#[test]
fn invalid_package_and_digest_requests_do_not_write() {
    let fixture = Fixture::new();
    for name in ["", "../escape", "missing"] {
        let mut identity = fixture.identity();
        identity.package = name.into();
        fixture.reject_unchanged(&identity);
    }
    for digest in ["".into(), "A".repeat(64), "0".repeat(63), "0".repeat(64)] {
        let mut identity = fixture.identity();
        identity.digest = digest;
        fixture.reject_unchanged(&identity);
    }
}

#[test]
fn tampered_cache_malformed_saved_data_and_unsupported_package_fail_closed() {
    for file in [
        "room.models.json",
        "room.scene.yaml",
        "orr.packages.lock.json",
        "orr.project.json",
    ] {
        let fixture = Fixture::new();
        let identity = fixture.identity();
        fs::write(fixture.root.join(file), b"malformed").unwrap();
        fixture.reject_unchanged(&identity);
    }
    let fixture = Fixture::new();
    let identity = fixture.identity();
    fs::write(
        fixture
            .root
            .join(".orr/packages/objects")
            .join(&identity.digest)
            .join(ASSET),
        b"tampered",
    )
    .unwrap();
    fixture.reject_unchanged(&identity);
    let fixture = Fixture::new();
    let identity = fixture.identity();
    fixture.install("unsupported", &["unavailable-runtime"]);
    fixture.reject_unchanged(&identity);
}

#[test]
fn editor_local_animation_feature_controls_the_entire_installed_lock() {
    let fixture = Fixture::new();
    let identity = fixture.identity();
    fixture.install("unused-animation", &["animation"]);
    let before = snapshot(fixture.temporary.path());
    // Querying a STATIC package must still reject an unsupported OTHER package.
    // Run additionally with only orr_sample/room-animated-models unified on.
    let result = inspect(&identity);
    assert_eq!(result.is_ok(), cfg!(feature = "room-animated-models"));
    assert_eq!(snapshot(fixture.temporary.path()), before);
}

#[cfg(feature = "room-animated-models")]
#[test]
fn animated_saved_rows_match_the_shared_query() {
    let fixture = Fixture::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let lock = Project::open_for_install(&fixture.root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();
    let asset = "animated.glb";
    let project =
        Project::open(&fixture.root, orr_sample::room_project::compiled_runtime()).unwrap();
    let loaded = model_bindings::load_asset_from_project(
        &project,
        "sample-animation",
        asset,
        ModelKind::Animated,
    )
    .unwrap();
    let path = fixture.root.join("room.models.json");
    let mut document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let binding = Binding::from_animated_asset(
        "sample-animation".into(),
        asset.into(),
        &loaded,
        model_bindings::AnimationDescriptor {
            clip_index: 0,
            playback: model_bindings::PlaybackMode::Loop,
        },
        LocalTransform::default(),
    )
    .unwrap();
    *document.bindings.values_mut().next().unwrap() = binding;
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
    let mut identity = fixture.identity();
    identity.package = "sample-animation".into();
    identity.digest = lock.packages["sample-animation"].digest.clone();
    let report = inspect(&identity).unwrap();
    let expected = authoring::inspect_usage(&authoring::Request {
        project: identity.root,
        package: identity.package,
        digest: identity.digest,
    })
    .unwrap();
    assert_eq!(report.bindings.len(), 1);
    assert_eq!(report.bindings[0].kind, ModelKind::Animated);
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn stale_completion_rejects_root_project_aba_package_digest_and_generation_changes() {
    let fixture = Fixture::new();
    fixture.install("unused", &["models"]);
    for mismatch in 0..7 {
        let mut panel = fixture.panel();
        completed(&mut panel, inspect(&fixture.identity()).unwrap());
        match mismatch {
            0 => panel.select_project(Some(Path::new("another-room"))),
            1 => {
                panel.select_project(Some(Path::new("another-room")));
                panel.select_project(Some(&fixture.root));
                panel.verified = Some(package_manager::inspect(&fixture.root).unwrap());
                panel.usage.select(PACKAGE.into());
            }
            2 => panel.project_generation += 1,
            3 => panel.usage.select("unused".into()),
            4 => {
                panel
                    .verified
                    .as_mut()
                    .unwrap()
                    .packages
                    .get_mut(PACKAGE)
                    .unwrap()
                    .digest = "0".repeat(64)
            }
            5 => panel.usage.invalidate(),
            _ => {
                panel.usage.select("unused".into());
                panel.usage.select(PACKAGE.into());
            }
        }
        panel.poll();
        assert!(!panel.busy());
        assert!(panel.usage.ready.is_none());
        assert!(panel.status.contains("discarded"));
    }
}

#[test]
fn dirty_modal_or_closed_panel_discards_completion_before_display() {
    let fixture = Fixture::new();
    for reason in [
        Some("Save changed Room files first"),
        Some("Close the modal first"),
        None,
    ] {
        let mut panel = fixture.panel();
        completed(&mut panel, inspect(&fixture.identity()).unwrap());
        if reason.is_none() {
            panel.open = false;
        }
        let root = fixture.root.clone();
        let mut harness = Harness::builder().build_ui_state(
            move |ui, panel: &mut Panel| panel.show(ui.ctx(), Some(&root), reason),
            panel,
        );
        harness.run_steps(2);
        assert!(harness.state().usage.ready.is_none());
        assert!(harness.state().status.contains("discarded"));
        assert!(!harness.state().busy());
    }
}

#[test]
fn query_uses_existing_busy_fence_without_mutation_freeze_or_repeated_dispatch() {
    let fixture = Fixture::new();
    let mut panel = fixture.panel();
    panel.start_usage();
    assert!(panel.busy());
    assert!(!panel.removal_busy());
    let generation = panel.usage.generation;
    panel.start_usage();
    panel.start_verify();
    panel.start_update_preview(fixture.source.clone());
    assert_eq!(panel.usage.generation, generation);
    wait(&mut panel);
    assert!(panel.usage.ready.is_some(), "{}", panel.status);
    assert!(!panel.removal_busy());
}

#[test]
fn verification_source_and_package_operations_invalidate_observation() {
    let fixture = Fixture::new();
    for operation in 0..10 {
        let mut panel = fixture.panel();
        panel.finish_usage(
            panel.usage_identity().unwrap(),
            inspect(&fixture.identity()),
        );
        assert!(panel.usage.ready.is_some());
        match operation {
            0 => panel.start_verify(),
            1 => panel.start_preview(fixture.source.clone()),
            2 => panel.start_update_preview(fixture.source.clone()),
            3 => panel.start_add(),
            4 => panel.start_remove(),
            5 => panel.start_removal_preview(),
            6 => panel.start_restoration_preview(),
            7 => panel.start_update(),
            8 => panel.invalidate_package_cache(),
            _ => {
                panel.source = "changed source".into();
                panel.select_source();
            }
        }
        assert!(panel.usage.ready.is_none());
        wait(&mut panel);
    }
}

#[test]
fn empty_report_never_creates_removal_or_update_authority() {
    let fixture = Fixture::new();
    fixture.install("unused", &["models"]);
    let mut panel = fixture.panel();
    panel.usage.select("unused".into());
    let before = snapshot(fixture.temporary.path());
    panel.start_usage();
    wait(&mut panel);
    let report = &panel.usage.ready.as_ref().unwrap().1;
    assert!(report.bindings.is_empty());
    assert!(report.warning.contains("not removal permission"));
    assert!(panel.removal_name.is_empty());
    assert!(panel.removal_preview.is_none());
    assert!(panel.update_preview.is_none());
    panel.start_remove();
    panel.start_update();
    assert!(!panel.busy());
    assert_eq!(snapshot(fixture.temporary.path()), before);
}

#[test]
fn usage_widgets_require_verification_and_explicit_inspection_then_render_every_row() {
    let fixture = Fixture::new();
    let before = snapshot(fixture.temporary.path());
    let mut panel = fixture.panel();
    panel.usage = State::default();
    let mut harness = Harness::builder()
        .with_size([1300.0, 1200.0])
        .build_ui_state(
            |ui, panel: &mut Panel| {
                panel.poll();
                panel.show_usage(ui, true);
            },
            panel,
        );
    harness.run_steps(2);
    assert!(harness
        .get_by_label("Inspect saved usage")
        .accesskit_node()
        .is_disabled());
    harness
        .get_by_label(&format!("Select usage {PACKAGE}"))
        .click();
    harness.run_steps(2);
    assert!(!harness.state().busy());
    assert!(harness.state().usage.ready.is_none());
    harness.get_by_label("Inspect saved usage").click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().usage.ready.is_none() {
        harness.run_steps(2);
        assert!(Instant::now() < deadline, "{}", harness.state().status);
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.run_steps(2);
    assert!(harness.query_by_label("Saved bindings: 2").is_some());
    for binding in &harness.state().usage.ready.as_ref().unwrap().1.bindings {
        assert!(harness
            .query_by_label(&format!("Entity: {}", binding.guid))
            .is_some());
        assert!(harness
            .query_by_label(&format!("Asset: {}", binding.asset))
            .is_some());
    }
    assert_eq!(snapshot(fixture.temporary.path()), before);
}

#[test]
fn live_editor_query_preserves_selection_checksum_history_and_binding_state() {
    let fixture = Fixture::new();
    let (_, path, scene, models) = PreparedProject::open(&fixture.root).unwrap().into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    let mut model_panel = ModelPanel::default();
    model_panel.install_room(models).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.sync();
        if editor.yard_rows_coherent() && model_panel.scene_matches(&editor) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let document = model_panel.bindings.as_ref().unwrap().document().clone();
    editor.select(Some(Target::Guid(
        Guid::parse(document.bindings.keys().next().unwrap()).unwrap(),
    )));
    let selection = editor.selection().cloned();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let disk = snapshot(fixture.temporary.path());
    let mut app = EditorApp::new(editor, None);
    app.models = model_panel;
    app.package_manager = fixture.panel();
    let mut harness = Harness::builder()
        .with_size([1400.0, 1000.0])
        .build_eframe(move |_| app);
    harness.run_steps(3);
    harness.state_mut().package_manager.start_usage();
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().package_manager.busy() {
        harness.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let app = harness.state();
    assert!(
        app.package_manager.usage.ready.is_some(),
        "{}",
        app.package_manager.status
    );
    assert_eq!(app.editor.selection(), selection.as_ref());
    assert_eq!(app.editor.history(), &history);
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &document);
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    assert_eq!(snapshot(fixture.temporary.path()), disk);
}

#[test]
fn verification_and_authoring_admission_are_required_before_dispatch() {
    let fixture = Fixture::new();
    let mut panel = fixture.panel();
    panel.verified = None;
    panel.start_usage();
    assert!(!panel.busy());
    assert!(panel.status.contains("Verify installed packages"));
    let mut harness = Harness::builder().build_ui_state(
        |ui, panel: &mut Panel| panel.show_usage(ui, false),
        fixture.panel(),
    );
    harness.run_steps(2);
    assert!(harness
        .get_by_label("Inspect saved usage")
        .accesskit_node()
        .is_disabled());
    harness
        .get_by_label("Inspect saved usage")
        .click_accesskit();
    harness.run_steps(2);
    assert!(!harness.state().busy());
    assert!(harness.state().usage.ready.is_none());
}

#[test]
fn failure_and_worker_disconnect_leave_no_report_or_mutation_authority() {
    let fixture = Fixture::new();
    for disconnect in [false, true] {
        let mut panel = fixture.panel();
        let (sender, receiver) = mpsc::channel();
        if !disconnect {
            sender
                .send(Completion::UsageInspected {
                    identity: panel.usage_identity().unwrap(),
                    result: Err("tampered saved state".into()),
                })
                .unwrap();
        }
        drop(sender);
        panel.pending = Some(receiver);
        panel.poll();
        assert!(!panel.busy());
        assert!(!panel.removal_busy());
        assert!(panel.usage.ready.is_none());
        assert!(panel.removal_preview.is_none());
        assert!(panel.update_preview.is_none());
        assert!(panel
            .status
            .contains(if disconnect { "disconnected" } else { "failed" }));
    }
}
