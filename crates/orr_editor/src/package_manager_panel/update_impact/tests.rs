//! Focused widget and stale-result coverage for read-only revision impact.
#![allow(clippy::disallowed_types)]
use super::*;
use crate::{model_panel::ModelPanel, Editor, EditorApp, HostSpec, Target};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::Guid;
use orr_sample::room_project::PreparedProject;
use orr_sample::room_project::PreparedScene;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};
const PACKAGE: &str = "impact-models";
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
        let manifest_path = source.join("orr.package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["version"] = "2.0.0".into();
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
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
        panel.source = self.source.display().to_string();
        panel.select_source();
        panel
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

fn enqueue(panel: &mut Panel, checked: bool) {
    let identity = panel.impact_identity().unwrap();
    let result = package_manager::inspect_update_impact(&identity.root, &identity.source);
    assert!(result.is_ok());
    let completion = if checked {
        Completion::UpdateImpactChecked { identity, result }
    } else {
        Completion::UpdateImpactInspected { identity, result }
    };
    let (sender, receiver) = mpsc::channel();
    sender.send(completion).unwrap();
    panel.pending = Some(receiver);
}

#[test]
fn impact_widgets_inspect_every_binding_without_authorizing_update_or_writing() {
    let fixture = Fixture::new();
    let before = snapshot(fixture.temporary.path());
    let mut harness = Harness::builder()
        .with_size([1300.0, 1200.0])
        .build_ui_state(
            |ui, panel: &mut Panel| {
                panel.poll();
                panel.show_update_impact(ui, true);
            },
            fixture.panel(),
        );
    harness.run_steps(2);
    harness.get_by_label("Inspect update impact").click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().update_impact.ready.is_none() {
        harness.run_steps(2);
        assert!(Instant::now() < deadline, "{}", harness.state().status);
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.run_steps(2);
    assert!(harness
        .query_by_label("Affected saved bindings: 2")
        .is_some());
    for binding in harness
        .state()
        .update_impact
        .ready
        .as_ref()
        .unwrap()
        .1
        .bindings()
    {
        assert!(harness
            .query_by_label(&format!("Impact entity: {}", binding.guid))
            .is_some());
        assert!(harness
            .query_by_label(&format!("Impact asset: {}", binding.asset))
            .is_some());
    }
    assert!(!harness.state().removal_busy());
    assert!(harness.state().update_preview.is_none());
    assert!(harness.state().removal_preview.is_none());
    assert!(package_manager::plan_update(&fixture.root, &fixture.source).is_err());
    assert_eq!(snapshot(fixture.temporary.path()), before);
}

#[test]
fn both_completion_phases_reject_project_source_aba_and_generation_changes() {
    let fixture = Fixture::new();
    for checked in [false, true] {
        for change in 0..6 {
            let mut panel = fixture.panel();
            enqueue(&mut panel, checked);
            match change {
                0 => panel.select_project(Some(Path::new("other"))),
                1 => {
                    panel.select_project(Some(Path::new("other")));
                    panel.select_project(Some(&fixture.root));
                }
                2 => {
                    panel.source = "other".into();
                    panel.select_source();
                }
                3 => {
                    panel.source = "other".into();
                    panel.select_source();
                    panel.source = fixture.source.display().to_string();
                    panel.select_source();
                }
                4 => panel.update_impact.invalidate(),
                _ => panel.project_generation += 1,
            }
            panel.poll();
            assert!(!panel.busy());
            assert!(panel.update_impact.ready.is_none());
            assert!(panel.status.contains("discarded"));
        }
    }
}

#[test]
fn changed_saved_files_or_source_while_first_result_queued_are_rejected() {
    for target in ["room.models.json", "orr.packages.lock.json", "source"] {
        let fixture = Fixture::new();
        let mut panel = fixture.panel();
        enqueue(&mut panel, false);
        if target == "source" {
            fs::write(fixture.source.join(ASSET), b"changed").unwrap();
        } else {
            fs::write(fixture.root.join(target), b"changed").unwrap();
        }
        let before = snapshot(fixture.temporary.path());
        wait(&mut panel);
        assert!(panel.update_impact.ready.is_none());
        assert!(panel.status.contains("discarded"), "{}", panel.status);
        assert_eq!(snapshot(fixture.temporary.path()), before);
    }
}

#[test]
fn closed_dirty_or_modal_panel_discards_both_completion_phases() {
    let fixture = Fixture::new();
    for checked in [false, true] {
        for reason in [Some("Save changes"), Some("Close modal"), None] {
            let mut panel = fixture.panel();
            enqueue(&mut panel, checked);
            if reason.is_none() {
                panel.open = false;
            }
            let root = fixture.root.clone();
            let mut harness = Harness::builder().build_ui_state(
                move |ui, panel: &mut Panel| panel.show(ui.ctx(), Some(&root), reason),
                panel,
            );
            harness.run_steps(2);
            assert!(harness.state().update_impact.ready.is_none());
            assert!(!harness.state().busy());
            assert!(harness.state().status.contains("discarded"));
        }
    }
}

#[test]
fn busy_dispatch_is_single_and_package_operations_invalidate_observation() {
    let fixture = Fixture::new();
    let mut panel = fixture.panel();
    panel.start_update_impact();
    assert!(panel.busy());
    assert!(!panel.removal_busy());
    let generation = panel.update_impact.generation;
    panel.start_update_impact();
    panel.start_verify();
    assert_eq!(panel.update_impact.generation, generation);
    wait(&mut panel);
    assert!(panel.update_impact.ready.is_some(), "{}", panel.status);
    for operation in 0..9 {
        let mut panel = fixture.panel();
        enqueue(&mut panel, true);
        panel.poll();
        assert!(panel.update_impact.ready.is_some());
        match operation {
            0 => panel.start_verify(),
            1 => panel.start_preview(fixture.source.clone()),
            2 => panel.start_update_preview(fixture.source.clone()),
            3 => panel.start_add(),
            4 => panel.start_remove(),
            5 => panel.start_removal_preview(),
            6 => panel.start_restoration_preview(),
            7 => panel.start_update(),
            _ => panel.invalidate_package_cache(),
        }
        assert!(panel.update_impact.ready.is_none());
        wait(&mut panel);
    }
}

#[test]
fn disabled_controls_failure_and_disconnect_never_create_report() {
    let fixture = Fixture::new();
    let mut harness = Harness::builder().build_ui_state(
        |ui, panel: &mut Panel| panel.show_update_impact(ui, false),
        fixture.panel(),
    );
    harness.run_steps(2);
    assert!(harness
        .get_by_label("Inspect update impact")
        .accesskit_node()
        .is_disabled());
    harness
        .get_by_label("Inspect update impact")
        .click_accesskit();
    harness.run_steps(2);
    assert!(!harness.state().busy());
    for disconnect in [false, true] {
        let mut panel = fixture.panel();
        let (sender, receiver) = mpsc::channel();
        if !disconnect {
            sender
                .send(Completion::UpdateImpactInspected {
                    identity: panel.impact_identity().unwrap(),
                    result: Err("bad source".into()),
                })
                .unwrap();
        }
        drop(sender);
        panel.pending = Some(receiver);
        panel.poll();
        assert!(!panel.busy());
        assert!(panel.update_impact.ready.is_none());
        assert!(panel.update_preview.is_none());
        assert!(panel
            .status
            .contains(if disconnect { "disconnected" } else { "failed" }));
    }
}

#[test]
fn live_editor_impact_preserves_selection_checksum_history_and_binding_state() {
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
    harness.state_mut().package_manager.start_update_impact();
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().package_manager.busy() {
        harness.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let app = harness.state();
    assert!(
        app.package_manager.update_impact.ready.is_some(),
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
