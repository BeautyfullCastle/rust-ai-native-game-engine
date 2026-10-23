use super::*;
use crate::mesh_navigation::{mesh_doc_from_yaml, MeshNavigation, PIN_NAME};
use crate::{Auth, Caps, ErpClient};
use bytemuck::Zeroable;
use orr_edit::{Op, Origin};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_navigation_runtime::mesh::AgentSpec;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

const EMPTY: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

fn document() -> EditorDoc {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO; 3],
            [fp!(4), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(4)],
            [fp!(4), FP::ZERO, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let pin = MeshScenePin::new(
        &graph,
        AgentSpec {
            start: [fp!(0.5), FP::ZERO, fp!(0.5)],
            goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap();
    let mut doc = hazards_doc_from_yaml(EMPTY).unwrap();
    let value = doc
        .types()
        .get(PIN_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&pin));
    doc.apply(
        Op::SetSingletonField {
            singleton: PIN_NAME.into(),
            path: String::new(),
            value,
        },
        Origin::User,
    )
    .unwrap();
    doc
}

fn config(doc: &EditorDoc) -> MeshHazards {
    let mut config = MeshHazards::zeroed();
    config.enabled = 1;
    config.count = 1;
    config.graph_revision = doc.frame().singleton::<MeshScenePin>().graph_revision;
    // The initial position is inside: the first Play tick must lose.
    config.spheres[0] = HazardSphere {
        center: doc.frame().singleton::<MeshScenePin>().start,
        radius: fp!(0.5),
    };
    config
}

fn apply(doc: &mut EditorDoc, config: &MeshHazards) {
    let value = doc
        .types()
        .get(HAZARDS_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(config));
    doc.apply(
        Op::SetSingletonField {
            singleton: HAZARDS_NAME.into(),
            path: String::new(),
            value,
        },
        Origin::User,
    )
    .unwrap();
}

#[test]
fn explicit_profile_registry_and_metadata_leave_legacy_hazard_free() {
    let legacy = mesh_doc_from_yaml(EMPTY).unwrap();
    assert!(legacy.types().get(HAZARDS_NAME).is_none());
    assert!(legacy.types().get(RUN_NAME).is_none());
    assert!(hazards_types().get(HAZARDS_NAME).is_some());
    assert_ne!(
        Simulation::<MeshNavigation>::build_registry().singleton_count(),
        Simulation::<MeshHazardNavigation>::build_registry().singleton_count()
    );
    let mut limits = HostLimits::default();
    configure_mesh_hazards(&mut limits);
    assert_eq!(limits.build_id, MESH_HAZARDS_BUILD_ID);
    assert_ne!(limits.build_id, super::super::MESH_NAVIGATION_BUILD_ID);
    let stream = limits.view_stream.as_ref().unwrap().lock();
    assert_eq!(stream.schema().game, MESH_HAZARDS_GAME);
    assert_eq!(stream.schema().build_id, MESH_HAZARDS_BUILD_ID);
}

#[test]
fn canonical_disabled_staging_and_invalid_config_fail_closed_atomically() {
    let mut doc = hazards_doc_from_yaml(EMPTY).unwrap();
    assert!(from_view(FrameView::of(doc.frame())).unwrap().is_none());
    let before = doc.to_yaml();
    assert!(doc
        .apply(
            Op::SetSingletonField {
                singleton: HAZARDS_NAME.into(),
                path: "enabled".into(),
                value: orr_reflect::Value::Int(1),
            },
            Origin::User
        )
        .is_err());
    assert_eq!(doc.to_yaml(), before);
    let mut doc = document();
    let good = config(&doc);
    apply(&mut doc, &good);
    let before = (doc.to_yaml(), doc.frame().to_bytes(), doc.revision());
    for (name, path, value) in [
        (RUN_NAME, "lost", 1),
        (RUN_NAME, "lost_tick", 1),
        (HAZARDS_NAME, "count", 9),
        (HAZARDS_NAME, "enabled", 2),
        (HAZARDS_NAME, "spheres[0].radius", 0),
    ] {
        assert!(
            doc.apply(
                Op::SetSingletonField {
                    singleton: name.into(),
                    path: path.into(),
                    value: orr_reflect::Value::Int(value),
                },
                Origin::User
            )
            .is_err(),
            "accepted {name}.{path}"
        );
        assert_eq!(
            (doc.to_yaml(), doc.frame().to_bytes(), doc.revision()),
            before
        );
    }
    let mut stale = good;
    stale.graph_revision[0] ^= 1;
    let view = mesh_from_view(FrameView::of(doc.frame())).unwrap();
    assert!(patch_request(&stale, &view).is_err());
    let request = patch_request(&good, &view).unwrap();
    assert_eq!(request["value"]["spheres"][0]["radius"], json!(0.5));
    assert!(
        mesh_doc_from_yaml(&doc.to_yaml()).is_err(),
        "legacy must not auto-migrate hazards"
    );
    let mut forged = doc.frame().clone();
    forged.set_singleton(MeshHazardRun {
        lost: 1,
        hit_index: 0,
        lost_tick: 1,
    });
    assert!(from_view(FrameView::of(&forged)).is_err());
}

#[test]
fn typed_hazard_view_reports_loss_and_rejects_inconsistent_runtime() {
    let mut doc = document();
    let config = config(&doc);
    apply(&mut doc, &config);
    let view = from_view(FrameView::of(doc.frame())).unwrap().unwrap();
    assert_eq!(view.config, config);
    assert_eq!(view.run, MeshHazardRun::zeroed());
    let mut sim = Simulation::<MeshHazardNavigation>::from_frame(
        doc.frame(),
        YARD_TICK_RATE,
        MESH_HAZARDS_BUILD_ID,
    )
    .unwrap();
    sim.step(&orr_sim::TickInputs::new(1, YARD_PLAYERS));
    let view = from_view(FrameView::of(sim.frame())).unwrap().unwrap();
    assert_eq!(view.run.lost, 1);
    assert_eq!(view.run.lost_tick, 1);
    let mut forged = sim.frame().clone();
    let state = *forged.singleton::<orr_navigation_runtime::mesh::RuntimeState>();
    let mut agent = *forged
        .get::<orr_navigation_runtime::mesh::RuntimeAgent>(state.agent)
        .unwrap();
    agent.status = orr_navigation::NavigationStatus::Moving as u32;
    forged.add(state.agent, agent);
    assert!(from_view(FrameView::of(&forged)).is_err());
}

struct Fixture {
    root: PathBuf,
    scene: PathBuf,
}
impl Fixture {
    fn new(doc: &EditorDoc) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "orr_hazards_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let scene = root.join("hazards.scene.yaml");
        fs::write(&scene, doc.to_yaml()).unwrap();
        Self { root, scene }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn yaml(client: &mut ErpClient) -> String {
    client.call("scene.save", json!({})).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string()
}
fn run(client: &mut ErpClient) -> JsonValue {
    client
        .call("world.singleton.get", json!({"name": RUN_NAME}))
        .unwrap()["value"]
        .clone()
}

#[test]
fn headless_patch_undo_redo_save_reload_play_loss_stop_and_reset() {
    let doc = document();
    let request = patch_request(
        &config(&doc),
        &mesh_from_view(FrameView::of(doc.frame())).unwrap(),
    )
    .unwrap();
    let fixture = Fixture::new(&doc);
    let mut settings = ServerConfig::new(Auth::DevNoAuth);
    settings.listen = false;
    let host = spawn_mesh_hazards_scene_host(fixture.scene.clone(), settings).unwrap();
    let mut client = ErpClient::with_transport(Box::new(
        host.connector().connect("hazards-test", Caps::ALL).unwrap(),
    ));
    let discovery = client.call("rpc.discover", json!({})).unwrap();
    assert_eq!(discovery["engine"]["game"], MESH_HAZARDS_GAME);
    let initial = yaml(&mut client);
    client.call("world.singleton.patch", request).unwrap();
    let mut edited = yaml(&mut client);
    assert_ne!(initial, edited);
    client.call("history.undo", json!({})).unwrap();
    assert_eq!(yaml(&mut client), initial);
    client.call("history.redo", json!({})).unwrap();
    assert_eq!(yaml(&mut client), edited);
    let history = client.call("history.list", json!({})).unwrap();
    assert!(client
        .call(
            "world.singleton.patch",
            json!({"name": RUN_NAME, "path": "lost", "value": 1})
        )
        .is_err());
    assert_eq!(client.call("history.list", json!({})).unwrap(), history);
    assert_eq!(yaml(&mut client), edited);
    // Apply a candidate through the same verified proposal path as agent clients.
    let id = client
        .call("proposal.begin", json!({"label": "widen hazard"}))
        .unwrap()["id"]
        .clone();
    client
        .call(
            "proposal.apply",
            json!({"id": id, "ops": [{
                "op": "singleton.patch", "name": HAZARDS_NAME,
                "path": "spheres[0].radius", "value": 0.75
            }]}),
        )
        .unwrap();
    assert_eq!(yaml(&mut client), edited);
    let report = client
        .call(
            "proposal.verify",
            json!({
                "id": id, "inputs": {"kind": "idle", "ticks": 2}, "sample_every": 1,
                "checks": ["entities.max == 1"]
            }),
        )
        .unwrap();
    assert_eq!(report["passed"], true);
    client
        .call(
            "proposal.accept_verified",
            json!({"id": id, "verified_state": report["verified_state"]}),
        )
        .unwrap();
    let previous = edited;
    edited = yaml(&mut client);
    assert_ne!(edited, previous);
    client.call("history.undo", json!({})).unwrap();
    assert_eq!(yaml(&mut client), previous);
    client.call("history.redo", json!({})).unwrap();
    assert_eq!(yaml(&mut client), edited);
    client.call("scene.save", json!({"write": true})).unwrap();
    let reopened = hazards_doc_from_path(&fixture.scene).unwrap();
    assert_eq!(reopened.to_yaml(), edited);
    assert_eq!(
        reopened.frame().to_bytes(),
        hazards_doc_from_yaml(&edited).unwrap().frame().to_bytes()
    );
    assert_eq!(run(&mut client)["lost"], 0);
    for _ in 0..2 {
        client.call("sim.start", json!({})).unwrap();
        assert_eq!(run(&mut client)["lost"], 0);
        client.call("sim.step", json!({"n": 1})).unwrap();
        assert_eq!(run(&mut client)["lost"], 1);
        assert_eq!(run(&mut client)["hit_index"], 0);
        assert_eq!(run(&mut client)["lost_tick"], 1);
        assert!(client
            .call(
                "world.singleton.patch",
                json!({"name": HAZARDS_NAME, "path": "count", "value": 0})
            )
            .is_err());
        client.call("sim.step", json!({"n": 3})).unwrap();
        assert_eq!(run(&mut client)["lost_tick"], 1);
        client.call("sim.stop", json!({})).unwrap();
        assert_eq!(run(&mut client)["lost"], 0);
        assert_eq!(yaml(&mut client), edited);
    }
    assert_eq!(fs::read_to_string(&fixture.scene).unwrap(), edited);
}

#[test]
fn hazards_path_reader_retains_size_bound() {
    let fixture = Fixture::new(&document());
    fs::write(&fixture.scene, vec![b' '; (1 << 20) + 1]).unwrap();
    assert!(hazards_doc_from_path(&fixture.scene)
        .err()
        .unwrap()
        .contains("1 MiB"));
}
