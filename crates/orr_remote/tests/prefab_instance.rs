//! `prefab_instance.*` boundary: capabilities, revisions, parameters and read-only calls.
#![cfg(feature = "linked-prefabs")]

use orr_edit::{EditorDoc, PlayController};
use orr_reflect::TypeRegistry;
use orr_remote::{call_local, Cap, Caps, ErpTarget, HostLimits, RpcError};
use orr_sample::physics_game::{register_reflect, PhysGame};
use orr_sim::Simulation;
use serde_json::{json, Value};

const SCENE: &str = include_str!("../../../scenes/joints_2d_demo.scene.yaml");
const SOURCE: &str = "motor.prefab.yaml";

struct Host {
    doc: EditorDoc,
    play: Option<PlayController<PhysGame>>,
    limits: HostLimits,
}

impl Host {
    fn new() -> Self {
        let mut types = TypeRegistry::new();
        register_reflect(&mut types);
        Self { doc: EditorDoc::from_yaml(SCENE, types, Simulation::<PhysGame>::build_registry(), 7).unwrap(), play: None, limits: HostLimits::default() }
    }

    fn call(&mut self, caps: Caps, method: &str, params: Value) -> Result<Value, RpcError> {
        call_local(&mut ErpTarget { doc: &mut self.doc, play: &mut self.play }, &self.limits, "prefab-test", caps, method, &params)
    }

    fn create(&mut self) -> Value {
        self.call(Caps::of(&[Cap::Read]), "prefab_instance.create", json!({"selected": ["e_00000007", "e_00000008", "e_00000009"]})).unwrap()
    }

    fn instantiate(&mut self, text: &str) -> Value {
        let revision = self.doc.revision();
        self.call(Caps::ALL, "prefab_instance.instantiate", json!({"source": SOURCE, "text": text, "expected_revision": revision})).unwrap()
    }
}

#[test]
fn read_only_calls_work_with_the_read_capability_and_change_nothing() {
    let mut host = Host::new();
    let before = (host.doc.to_yaml(), host.doc.revision(), host.doc.history().len());
    let created = host.create();
    assert!(created["text"].as_str().unwrap().contains("e_00000003"));
    assert_eq!(created["guids"]["e_00000003"], "e_00000009");
    let read = Caps::of(&[Cap::Read]);
    assert_eq!(host.call(read, "prefab_instance.list", json!({})).unwrap()["instances"], json!([]));
    assert_eq!(host.call(read, "prefab_instance.entity", json!({"entity": "e_00000007"})).unwrap()["member"], false);
    assert_eq!((host.doc.to_yaml(), host.doc.revision(), host.doc.history().len()), before);
}

#[test]
fn every_mutator_needs_scene_edit_the_exact_revision_and_known_parameters() {
    let mut host = Host::new();
    let text = host.create()["text"].as_str().unwrap().to_string();
    let revision = host.doc.revision();
    let read = Caps::of(&[Cap::Read]);
    for (method, params) in [
        ("prefab_instance.instantiate", json!({"source": SOURCE, "text": text, "expected_revision": revision})),
        ("prefab_instance.update", json!({"source": SOURCE, "text": text, "expected_revision": revision})),
        ("prefab_instance.detach", json!({"entity": "e_00000007", "expected_revision": revision})),
        ("prefab_instance.revert", json!({"entity": "e_00000007", "expected_revision": revision})),
    ] {
        let error = host.call(read, method, params).unwrap_err();
        assert_eq!(error.code, orr_remote::PERMISSION_DENIED, "{method}: {error:?}");
    }
    let before = (host.doc.to_yaml(), host.doc.revision());
    let stale = host.call(Caps::ALL, "prefab_instance.instantiate", json!({"source": SOURCE, "text": text, "expected_revision": revision + 1}));
    assert!(format!("{stale:?}").contains("stale_revision"), "{stale:?}");
    let unknown = host.call(Caps::ALL, "prefab_instance.instantiate", json!({"source": SOURCE, "text": text, "expected_revision": revision, "surprise": 1}));
    assert!(format!("{unknown:?}").contains("unknown prefab_instance parameter"), "{unknown:?}");
    let missing = host.call(Caps::ALL, "prefab_instance.instantiate", json!({"source": SOURCE, "text": text}));
    assert!(missing.is_err());
    assert_eq!((host.doc.to_yaml(), host.doc.revision()), before, "rejected calls change nothing");
}

#[test]
fn instantiate_update_revert_and_detach_round_trip_through_the_methods() {
    let mut host = Host::new();
    let text = host.create()["text"].as_str().unwrap().to_string();
    let first = host.instantiate(&text);
    let second = host.instantiate(&text);
    assert_eq!(second["instances"].as_array().unwrap().len(), 2);
    let pin = first["guids"]["e_00000003"].as_str().unwrap().to_string();
    let propeller = first["guids"]["e_00000002"].as_str().unwrap().to_string();
    assert_ne!(pin, "e_00000009");

    // An override shows up in the list and in the entity query.
    let revision = host.doc.revision();
    host.call(Caps::ALL, "world.patch", json!({"entity": propeller, "component": "orr_physics::Body", "path": "angle", "value": 2})).unwrap();
    let entity = host.call(Caps::of(&[Cap::Read]), "prefab_instance.entity", json!({"entity": propeller})).unwrap();
    assert_eq!(entity["member"], true);
    assert_eq!(entity["overrides"][0]["kind"], "field");
    assert_eq!(entity["overrides"][0]["path"], "angle");
    assert!(host.doc.revision() > revision);

    // Revert one field, then detach and undo.
    let revision = host.doc.revision();
    let reverted = host.call(Caps::ALL, "prefab_instance.revert", json!({"entity": propeller, "component": "orr_physics::Body", "path": "angle", "expected_revision": revision})).unwrap();
    assert_eq!(reverted["reverted"], 1);
    let history = host.doc.history().len();
    let revision = host.doc.revision();
    let detached = host.call(Caps::ALL, "prefab_instance.detach", json!({"entity": propeller, "expected_revision": revision})).unwrap();
    assert_eq!(detached["instances"].as_array().unwrap().len(), 1);
    assert_eq!(host.doc.history().len(), history + 1);
    host.doc.undo().unwrap();
    assert_eq!(host.call(Caps::of(&[Cap::Read]), "prefab_instance.list", json!({})).unwrap()["instances"].as_array().unwrap().len(), 2);

    // Reverting something that is not overridden is an error, not a silent no-op.
    let revision = host.doc.revision();
    let error = host.call(Caps::ALL, "prefab_instance.revert", json!({"entity": propeller, "component": "orr_physics::Body", "path": "angle", "expected_revision": revision}));
    assert!(error.is_err());
}

#[test]
fn methods_are_discoverable() {
    let mut host = Host::new();
    let discovered = host.call(Caps::of(&[Cap::Read]), "rpc.discover", json!({})).unwrap();
    let names: Vec<&str> = discovered["methods"].as_array().unwrap().iter().filter_map(|m| m["name"].as_str()).filter(|n| n.starts_with("prefab_instance.")).collect();
    assert_eq!(names.len(), 9, "{names:?}");
}
