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
        Self {
            _temp: temp,
            root,
            source,
            dependency,
        }
    }
    fn panel(&self) -> Panel {
        let mut p = Panel {
            open: true,
            ..Default::default()
        };
        p.select_project(Some(&self.root));
        p.source = self.source.display().to_string();
        p.select_source();
        p.dependency_add.candidates = self.dependency.display().to_string();
        p.dependency_add.observe();
        p
    }
    fn preview(&self, p: &mut Panel) {
        let plan = package_manager::plan_add_with_dependencies(
            &self.root,
            &self.source,
            std::slice::from_ref(&self.dependency),
        )
        .unwrap();
        p.dependency_add.ready = Some((p.dependency_identity().unwrap(), plan));
    }
}
fn wait(p: &mut Panel) {
    let end = Instant::now() + Duration::from_secs(10);
    while p.busy() {
        p.poll();
        assert!(Instant::now() < end, "{}", p.status);
        std::thread::yield_now();
    }
}
#[test]
fn candidate_text_bounds_and_exact_generation_track_aba() {
    for text in [
        "".to_owned(),
        "x\n".repeat(9),
        "x".repeat(4097),
        " ".repeat(32769),
    ] {
        assert!(candidates(&text).is_err());
    }
    assert_eq!(
        candidates(" a \n\nb\n").unwrap(),
        [PathBuf::from("a"), PathBuf::from("b")]
    );
    let mut s = State {
        candidates: "a".into(),
        ..State::default()
    };
    s.observe();
    let generation = s.generation;
    s.candidates = "b".into();
    s.observe();
    s.candidates = "a".into();
    s.observe();
    assert!(s.generation > generation);
}
#[test]
fn ready_completion_rejects_source_project_candidate_and_aba_changes() {
    let f = Fixture::new();
    for change in 0..6 {
        let mut p = f.panel();
        f.preview(&mut p);
        let (identity, plan) = p.dependency_add.ready.take().unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Completion::DependenciesPreviewed {
            identity,
            result: Ok(plan),
        })
        .unwrap();
        p.pending = Some(rx);
        match change {
            0 => p.select_project(Some(Path::new("other"))),
            1 => {
                p.source = "different".into();
                p.select_source();
            }
            2 => p.dependency_add.candidates.push(' '),
            3 => p.project_generation += 1,
            4 => p.source_generation += 1,
            _ => {
                let old = p.dependency_add.candidates.clone();
                p.dependency_add.candidates = "different".into();
                p.dependency_add.observe();
                p.dependency_add.candidates = old;
            }
        }
        p.poll();
        assert!(p.dependency_add.ready.is_none());
        assert!(p.status.contains("discarded"));
    }
}
#[test]
fn blocked_and_closed_panel_invalidate_before_ready_completion_poll() {
    let f = Fixture::new();
    for closed in [false, true] {
        let mut p = f.panel();
        f.preview(&mut p);
        let (identity, plan) = p.dependency_add.ready.take().unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Completion::DependenciesPreviewed {
            identity,
            result: Ok(plan),
        })
        .unwrap();
        p.pending = Some(rx);
        p.open = !closed;
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            p.show(
                ui.ctx(),
                Some(&f.root),
                if closed { None } else { Some("Save first") },
            )
        });
        assert!(p.dependency_add.ready.is_none());
        assert!(!p.busy());
        assert!(p.status.contains("discarded"));
    }
}
#[test]
fn shared_busy_serializes_repeated_clicks_and_old_add_update_lanes() {
    let f = Fixture::new();
    let mut p = f.panel();
    p.start_dependencies_preview();
    assert!(p.busy());
    p.start_dependencies_preview();
    p.start_dependencies_add();
    p.start_preview(f.source.clone());
    p.start_add();
    p.start_update();
    assert!(p.busy());
    wait(&mut p);
    assert!(p.dependency_add.ready.is_some(), "{}", p.status);
    let before = fs::read(f.root.join("orr.packages.lock.json")).unwrap();
    assert!(!package_manager::inspect(&f.root)
        .unwrap()
        .packages
        .contains_key("a"));
    // An existing worker blocks the new lane too, without consuming its ticket.
    let (tx, rx) = mpsc::channel();
    p.pending = Some(rx);
    p.start_dependencies_add();
    assert!(p.dependency_add.ready.is_some());
    drop(tx);
    p.poll();
    assert_eq!(
        fs::read(f.root.join("orr.packages.lock.json")).unwrap(),
        before
    );
}
#[test]
fn dispatched_install_is_fenced_and_not_cancelled_by_close_or_project_change() {
    for change_project in [false, true] {
        let f = Fixture::new();
        let mut p = f.panel();
        f.preview(&mut p);
        p.start_dependencies_add();
        assert!(p.removal_busy());
        assert!(p.dependency_add.ready.is_none());
        p.start_dependencies_add();
        p.open = false;
        let target = if change_project {
            Path::new("other-project")
        } else {
            &f.root
        };
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            p.show(ui.ctx(), Some(target), Some("Package mutation pending"))
        });
        wait(&mut p);
        assert!(!p.removal_busy());
        assert!(
            p.status.contains(if change_project {
                "prior project"
            } else {
                "current project"
            }),
            "{}",
            p.status
        );
        assert!(p.verified.is_none());
        assert!(package_manager::inspect(&f.root)
            .unwrap()
            .packages
            .contains_key("a"));
    }
}
#[test]
fn disconnect_or_failure_never_retains_preview_or_claims_rollback() {
    let f = Fixture::new();
    for disconnect in [false, true] {
        let mut p = f.panel();
        f.preview(&mut p);
        let identity = p.dependency_identity().unwrap();
        let (tx, rx) = mpsc::channel();
        p.pending = Some(rx);
        p.installing_dependencies = true;
        if !disconnect {
            tx.send(Completion::DependenciesAdded {
                identity,
                result: Err("post-publication uncertainty".into()),
            })
            .unwrap();
        }
        drop(tx);
        assert!(p.removal_busy());
        p.poll();
        assert!(!p.removal_busy());
        assert!(p.dependency_add.ready.is_none());
        assert!(p.verified.is_none());
        assert!(p.status.contains("no automatic retry or rollback"));
        if disconnect {
            assert!(p.status.contains("publication outcome is uncertain"));
        }
    }
}
