// Wall-clock deadlines supervise only test worker completion, not simulation.
#![allow(clippy::disallowed_types)]
use super::*;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    dependency: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let root = temp.path().join("room");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("room.scene.yaml"),
            include_str!("../../../tests/fixtures/room_initial.scene.yaml"),
        )
        .unwrap();
        fs::write(root.join("orr.project.json"), br#"{"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}}"#).unwrap();
        // Install a real baseline model through the package core before Room
        // admission. Empty bindings are not a valid prepared Room fixture.
        let baseline = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .unwrap();
        orr_package::Project::open_for_install(
            &root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[baseline])
        .unwrap();
        let scene = orr_sample::room_project::PreparedScene::parse(include_str!(
            "../../../tests/fixtures/room_initial.scene.yaml"
        ))
        .unwrap();
        let player = scene
            .frame()
            .entities()
            .find(|entity| {
                scene
                    .frame()
                    .get::<orr_sample::room_game::RoomActor>(*entity)
                    .is_some_and(|actor| actor.kind == orr_sample::room_game::PLAYER)
            })
            .unwrap();
        let asset = orr_model_bindings::model_bindings::load_asset(
            &root,
            "sample-imported-scene",
            "foreground.glb",
        )
        .unwrap();
        let binding = orr_model_bindings::model_bindings::Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &asset,
            orr_model_bindings::model_bindings::LocalTransform::default(),
        )
        .unwrap();
        let document = orr_model_bindings::model_bindings::Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: [(scene.index().guid(player).unwrap().to_string(), binding)]
                .into_iter()
                .collect(),
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        package_manager::inspect(&root).unwrap();
        let source = temp.path().join("a");
        let dependency = temp.path().join("b");
        for (path, name, deps) in [
            (&source, "a", serde_json::json!({"b":"1.0.0"})),
            (&dependency, "b", serde_json::json!({})),
        ] {
            fs::create_dir(path).unwrap();
            fs::write(path.join("LICENSE.txt"), b"Fixture").unwrap();
            fs::write(path.join("orr.package.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"name":name,"version":"1.0.0","engine":"*","capabilities":[],"dependencies":deps,"files":["LICENSE.txt"]})).unwrap()).unwrap();
        }
        let old = temp.path().join("old-a");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("LICENSE.txt"), b"Old fixture").unwrap();
        fs::write(old.join("orr.package.json"), serde_json::to_vec(&serde_json::json!({"schema":1,"name":"a","version":"1.0.0","engine":"*","capabilities":[],"dependencies":{},"files":["LICENSE.txt"]})).unwrap()).unwrap();
        package_manager::apply_add(&package_manager::plan_add(&root, &old).unwrap()).unwrap();
        let path = source.join("orr.package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["version"] = serde_json::json!("2.0.0");
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        Self {
            _temp: temp,
            root,
            source,
            dependency,
        }
    }
    fn harness(&self) -> Harness<'static, Panel> {
        let mut p = Panel {
            open: true,
            ..Default::default()
        };
        p.select_project(Some(&self.root));
        p.source = self.source.display().to_string();
        p.select_source();
        let mut h = Harness::builder()
            .with_size([1000.0, 900.0])
            .build_ui_state(
                |ui, p: &mut Panel| {
                    p.show_dependency_add(ui, true, false);
                    p.show_dependency_update(ui, true);
                },
                p,
            );
        h.run_steps(2);
        h.get_by_role_and_label(
            egui::accesskit::Role::MultilineTextInput,
            "Dependency candidate directories (one per line)",
        ).focus();
        h.run_steps(2);
        h.get_by_role_and_label(
            egui::accesskit::Role::MultilineTextInput,
            "Dependency candidate directories (one per line)",
        )
        .type_text(&self.dependency.display().to_string());
        h.run_steps(2);
        assert_eq!(h.state().dependency_add.candidate_identity().0, self.dependency.to_str().unwrap());
        assert_eq!(h.state().source, self.source.to_str().unwrap());
        h
    }
    fn preview(&self, p: &mut Panel) {
        let plan = package_manager::plan_update_with_dependencies(
            &self.root,
            &self.source,
            std::slice::from_ref(&self.dependency),
        )
        .unwrap();
        p.dependency_update.ready = Some((p.dependency_update_identity().unwrap(), plan));
    }
}
use egui_kittest::{kittest::Queryable, Harness};
fn wait(p: &mut Panel) {
    let end = Instant::now() + Duration::from_secs(10);
    while p.busy() {
        p.poll();
        assert!(Instant::now() < end, "{}", p.status);
        std::thread::yield_now();
    }
}
#[test]
fn candidate_text_aba_revokes_update_even_when_shared_observe_ran_first() {
    let f = Fixture::new();
    let mut h = f.harness();
    f.preview(h.state_mut());
    let original = h.state().dependency_update_identity().unwrap();
    let input = h.get_by_role_and_label(
        egui::accesskit::Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    );
    input.focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(
        egui::accesskit::Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    )
    .type_text("changed");
    h.run_steps(2);
    h.get_by_role_and_label(
        egui::accesskit::Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    )
    .focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(
        egui::accesskit::Role::MultilineTextInput,
        "Dependency candidate directories (one per line)",
    )
    .type_text(&f.dependency.display().to_string());
    h.run_steps(2);
    assert_ne!(h.state().dependency_update_identity().unwrap(), original);
    h.state_mut().start_dependencies_update();
    assert!(!h.state().busy());
    assert!(h.state().dependency_update.ready.is_none());
    assert_eq!(
        package_manager::inspect(&f.root).unwrap().packages["a"]
            .manifest
            .version,
        "1.0.0"
    );
}
#[test]
fn preview_completion_rejects_project_source_close_and_block_changes() {
    let f = Fixture::new();
    for change in 0..5 {
        let mut h = f.harness();
        f.preview(h.state_mut());
        let p = h.state_mut();
        let (identity, plan) = p.dependency_update.ready.take().unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Completion::DependenciesUpdatePreviewed {
            identity,
            result: Ok(plan),
        })
        .unwrap();
        p.pending = Some(rx);
        match change {
            0 => p.select_project(Some(Path::new("other"))),
            1 => {
                p.source = "other".into();
                p.select_source();
            }
            2 => p.project_generation += 1,
            _ => {}
        }
        if change >= 3 {
            p.open = change != 3;
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(Default::default(), |ui| {
                p.show(
                    ui.ctx(),
                    Some(&f.root),
                    if change == 4 {
                        Some("Save first")
                    } else {
                        None
                    },
                )
            });
        } else {
            p.poll();
        }
        assert!(p.dependency_update.ready.is_none());
        assert!(p.status.contains("discarded"));
    }
}
#[test]
fn update_preview_and_add_intents_are_mutually_exclusive_and_share_busy() {
    let f = Fixture::new();
    let mut h = f.harness();
    f.preview(h.state_mut());
    let before = h.state().dependency_add.candidate_identity().1;
    h.state_mut().start_dependencies_preview();
    assert!(h.state().dependency_update.ready.is_none());
    h.state_mut().start_dependencies_update_preview();
    assert!(h.state().busy());
    wait(h.state_mut());
    // Add preview correctly rejects installed a; this does not manufacture an Update plan.
    assert!(h.state().dependency_update.ready.is_none());
    h.get_by_label("Preview package update with dependencies")
        .click_accesskit();
    h.run_steps(1);
    assert!(h.state().busy());
    h.state_mut().start_dependencies_update_preview();
    h.state_mut().start_add();
    h.state_mut().start_update();
    wait(h.state_mut());
    assert!(
        h.state().dependency_update.ready.is_some(),
        "{}",
        h.state().status
    );
    assert!(h.state().dependency_add.candidate_identity().1 > before);
    let lock = package_manager::inspect(&f.root).unwrap();
    assert_eq!(lock.packages["a"].manifest.version, "1.0.0");
}
#[test]
fn update_dispatch_fence_survives_close_block_and_prior_project() {
    for prior in [false, true] {
        let f = Fixture::new();
        let mut h = f.harness();
        f.preview(h.state_mut());
        let p = h.state_mut();
        p.start_dependencies_update();
        assert!(p.removal_busy());
        p.start_dependencies_update();
        p.open = false;
        let target = if prior { Path::new("other") } else { &f.root };
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            p.show(ui.ctx(), Some(target), Some("Mutation pending"))
        });
        wait(p);
        assert!(
            p.status.contains(if prior {
                "prior project"
            } else {
                "current project"
            }),
            "{}",
            p.status
        );
        assert!(p.verified.is_none());
        assert!(!p.removal_busy());
        assert_eq!(
            package_manager::inspect(&f.root).unwrap().packages["a"]
                .manifest
                .version,
            "2.0.0"
        );
    }
}
#[test]
fn update_disconnect_and_uncertain_completion_discard_all_tickets() {
    let f = Fixture::new();
    for disconnected in [false, true] {
        let mut h = f.harness();
        f.preview(h.state_mut());
        let p = h.state_mut();
        let identity = p.dependency_update_identity().unwrap();
        let (tx, rx) = mpsc::channel();
        p.pending = Some(rx);
        p.updating_dependencies = true;
        if !disconnected {
            tx.send(Completion::DependenciesUpdated {
                identity,
                result: Err("post-publication uncertainty".into()),
            })
            .unwrap();
        }
        drop(tx);
        p.poll();
        assert!(p.dependency_update.ready.is_none());
        assert!(!p.removal_busy());
        assert!(p.verified.is_none());
        assert!(p.status.contains("no automatic retry or rollback"));
    }
}
