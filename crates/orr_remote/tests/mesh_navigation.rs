#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types)]

use orr_bridge::FrameView;
use orr_ecs::Frame;
use orr_edit::{EditorDoc, Op, Origin};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_navigation_runtime::mesh::{
    AgentSpec, AssetByte, MeshRuntime, RuntimeState, MAX_GRAPH_BYTES,
};
use orr_reflect::Value;
use orr_remote::mesh_navigation::*;
use orr_remote::{Auth, Caps, ErpClient, LocalHost, ServerConfig};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value as Json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const EMPTY_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

struct Fixture {
    root: PathBuf,
    scene: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "orr_mesh_navigation_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            scene: root.join("mesh.scene.yaml"),
            root,
        }
    }

    fn document(&self) -> EditorDoc {
        mesh_doc_from_path(&self.scene).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn agent_spec() -> AgentSpec {
    AgentSpec {
        start: [fp!(0.25), FP::ZERO, fp!(0.25)],
        goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    }
}

fn document_with_pin() -> EditorDoc {
    let mut document = mesh_doc_from_yaml(EMPTY_SCENE).unwrap();
    let pin = MeshScenePin::new(&graph(), agent_spec()).unwrap();
    let value = document
        .types()
        .get(PIN_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&pin));
    document
        .apply(
            Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: String::new(),
                value,
            },
            Origin::User,
        )
        .unwrap();
    document
}

fn client(host: &LocalHost) -> ErpClient {
    ErpClient::with_transport(Box::new(
        host.connector().connect("user", Caps::ALL).unwrap(),
    ))
}

fn scene_text(client: &mut ErpClient) -> String {
    client.call("scene.save", Json::Null).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string()
}

fn snapshot(document: &EditorDoc) -> (String, Vec<u8>, u64, bool, bool, bool) {
    (
        document.to_yaml(),
        document.frame().to_bytes(),
        document.revision(),
        document.is_dirty(),
        document.can_undo(),
        document.can_redo(),
    )
}

#[test]
fn embedded_pin_admits_one_agent_and_reopens_without_external_assets() {
    let mut document = document_with_pin();
    assert!(document.scene().entities.is_empty());
    assert_eq!(document.frame().alive_count(), 1);
    assert_eq!(
        document
            .frame()
            .count::<orr_navigation_runtime::mesh::RuntimeAgent>(),
        1
    );
    let view = mesh_from_view(FrameView::of(document.frame())).unwrap();
    assert_eq!(view.graph.revision(), view.pin.graph_revision);
    assert_eq!(view.agent.spec, view.pin.spec());
    assert_eq!(view.agent.binding, view.pin.binding());
    MeshRuntime::validate(document.frame(), view.pin.binding()).unwrap();

    let saved = document.save_yaml();
    let reopened = mesh_doc_from_yaml(&saved).unwrap();
    assert_eq!(document.frame().to_bytes(), reopened.frame().to_bytes());

    let mut stale_asset = Frame::from_bytes(
        document.frame_registry().clone(),
        &document.frame().to_bytes(),
    )
    .unwrap();
    let state = *stale_asset.singleton::<RuntimeState>();
    stale_asset.list_free(state.graph_bytes);
    assert!(mesh_from_view(FrameView::of(&stale_asset)).is_err());

    let mut oversized_asset = Frame::from_bytes(
        document.frame_registry().clone(),
        &document.frame().to_bytes(),
    )
    .unwrap();
    let state = *oversized_asset.singleton::<RuntimeState>();
    for _ in 0..=MAX_GRAPH_BYTES {
        oversized_asset.list_push(state.graph_bytes, AssetByte { value: 0 });
    }
    assert!(mesh_from_view(FrameView::of(&oversized_asset)).is_err());

    let mut extra_entity = Frame::from_bytes(
        document.frame_registry().clone(),
        &document.frame().to_bytes(),
    )
    .unwrap();
    extra_entity.spawn();
    assert!(mesh_from_view(FrameView::of(&extra_entity)).is_err());

    let mut noncanonical_status = Frame::from_bytes(
        document.frame_registry().clone(),
        &document.frame().to_bytes(),
    )
    .unwrap();
    let mut status = *noncanonical_status.singleton::<MeshStepStatus>();
    status.error[0] = 1;
    noncanonical_status.set_singleton(status);
    assert!(mesh_from_view(FrameView::of(&noncanonical_status)).is_err());
}

#[test]
fn empty_pin_is_edit_staging_but_cannot_silently_run() {
    let document = mesh_doc_from_yaml(EMPTY_SCENE).unwrap();
    assert_eq!(document.frame().alive_count(), 0);
    assert!(mesh_from_view(FrameView::of(document.frame())).is_err());
    let mut simulation = Simulation::<MeshNavigation>::from_frame(
        document.frame(),
        YARD_TICK_RATE,
        MESH_NAVIGATION_BUILD_ID,
    )
    .unwrap();
    simulation.step(&TickInputs::new(1, YARD_PLAYERS));
    let status = simulation.frame().singleton::<MeshStepStatus>();
    assert_eq!(status.failed, 1);
    assert_eq!(status.failed_tick, 1);
    assert_eq!(simulation.frame().alive_count(), 0);
}

#[test]
fn erp_pin_patch_undo_redo_invalid_admission_save_and_play_share_one_authority() {
    let fixture = Fixture::new();
    let document = document_with_pin();
    let initial_frame = document.frame().to_bytes();
    fs::write(&fixture.scene, document.to_yaml()).unwrap();

    let mut config = ServerConfig::new(Auth::DevNoAuth);
    config.listen = false;
    let host = spawn_mesh_navigation_scene_host(fixture.scene.clone(), config).unwrap();
    let mut client = client(&host);
    assert_eq!(
        client.call("rpc.discover", Json::Null).unwrap()["engine"]["game"],
        "MeshNavigation"
    );
    let initial_yaml = scene_text(&mut client);
    let initial_history = client.call("history.list", Json::Null).unwrap();
    assert_eq!(initial_history["entries"].as_array().unwrap().len(), 0);

    client
        .call(
            "world.singleton.patch",
            json!({"name": PIN_NAME, "path": "distance_per_tick", "value": "0.5"}),
        )
        .unwrap();
    let edited_yaml = scene_text(&mut client);
    assert_ne!(edited_yaml, initial_yaml);
    assert_eq!(
        client.call("history.list", Json::Null).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    client.call("history.undo", Json::Null).unwrap();
    assert_eq!(scene_text(&mut client), initial_yaml);
    client.call("history.redo", Json::Null).unwrap();
    assert_eq!(scene_text(&mut client), edited_yaml);

    let edited_document = mesh_doc_from_yaml(&edited_yaml).unwrap();
    let current_pin = *edited_document.frame().singleton::<MeshScenePin>();
    let mismatched_revision = (u16::from(current_pin.graph_revision[0]) + 1) % 256;
    let before_invalid = client.call("history.list", Json::Null).unwrap();
    assert!(client
        .call(
            "world.singleton.patch",
            json!({
                "name": PIN_NAME,
                "path": "graph_revision[0]",
                "value": mismatched_revision
            }),
        )
        .is_err());
    assert_eq!(scene_text(&mut client), edited_yaml);
    assert_eq!(
        client.call("history.list", Json::Null).unwrap(),
        before_invalid
    );

    client.call("sim.start", Json::Null).unwrap();
    let before_play = client.call("sim.state", Json::Null).unwrap();
    client.call("sim.step", json!({"n": 4})).unwrap();
    let after_play = client.call("sim.state", Json::Null).unwrap();
    assert_ne!(before_play["checksum"], after_play["checksum"]);
    client.call("sim.stop", Json::Null).unwrap();

    client.call("scene.save", json!({"write": true})).unwrap();
    let saved_yaml = fs::read_to_string(&fixture.scene).unwrap();
    assert_eq!(saved_yaml, edited_yaml);
    let reopened = fixture.document();
    assert_eq!(reopened.to_yaml(), saved_yaml);
    assert_eq!(
        reopened.frame().to_bytes(),
        edited_document.frame().to_bytes()
    );
    assert_ne!(initial_frame, reopened.frame().to_bytes());
}

#[test]
fn invalid_embedded_graph_pin_fails_atomically_during_rebake() {
    let mut document = document_with_pin();
    let before = snapshot(&document);
    let pin = *document.frame().singleton::<MeshScenePin>();
    let mismatched_revision = (u16::from(pin.graph_revision[0]) + 1) % 256;
    assert!(document
        .apply(
            Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: "graph_revision[0]".into(),
                value: Value::Int(i128::from(mismatched_revision)),
            },
            Origin::User,
        )
        .is_err());
    assert_eq!(snapshot(&document), before);
}

#[test]
fn oversized_scene_source_is_rejected_before_parsing() {
    let fixture = Fixture::new();
    fs::write(&fixture.scene, vec![b' '; (1 << 20) + 1]).unwrap();
    let error = mesh_doc_from_path(&fixture.scene)
        .err()
        .expect("oversized source is rejected");
    assert!(error.contains("1 MiB"));
    assert_eq!(
        fs::metadata(&fixture.scene).unwrap().len(),
        ((1 << 20) + 1) as u64
    );
}
