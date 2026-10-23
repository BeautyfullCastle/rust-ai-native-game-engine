use crate::input::StructuredInput;
use crate::mesh_navigation::{spawn_mesh_navigation_scene_host, MeshNavigation, YARD_PLAYERS};
use crate::{Auth, Caps, ErpClient, ServerConfig};
use orr_games::yard3d_game::YardInput;
use orr_sim::PlayerSlot;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const EMPTY_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

struct Fixture {
    root: PathBuf,
    scene: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "orr_mesh_play_input_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let scene = root.join("empty.mesh.yaml");
        fs::write(&scene, EMPTY_SCENE).unwrap();
        Self { root, scene }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn input_adapter() -> StructuredInput<MeshNavigation> {
    StructuredInput::new("MeshGoalInput", YARD_PLAYERS, |_, _| Vec::new())
}

fn client(host: &crate::LocalHost) -> ErpClient {
    ErpClient::with_transport(Box::new(
        host.connector()
            .connect("mesh-play-input-test", Caps::ALL)
            .unwrap(),
    ))
}

fn goal_value() -> Value {
    json!({
        "buttons": 1u32 << 31,
        "origin": [32768, 0, -32768],
        "dir": [0, 0, 0]
    })
}

#[test]
fn mesh_goal_descriptor_hides_padding_and_preserves_yard_input_layout() {
    let adapter = input_adapter();
    let descriptor = &adapter.descriptor;
    let properties = descriptor["schema"]["properties"].as_object().unwrap();
    assert_eq!(descriptor["schema"]["title"], "MeshGoalInput");
    assert_eq!(descriptor["max_players"], YARD_PLAYERS);
    assert_eq!(properties.len(), 3);
    assert!(properties.contains_key("buttons"));
    assert!(properties.contains_key("origin"));
    assert!(properties.contains_key("dir"));
    assert!(!properties.contains_key("_pad"));
    assert_eq!(properties["buttons"]["minimum"], 0);
    assert_eq!(properties["buttons"]["maximum"], u32::MAX);
    for field in ["origin", "dir"] {
        assert_eq!(properties[field]["type"], "array");
        assert_eq!(properties[field]["minItems"], 3);
        assert_eq!(properties[field]["maxItems"], 3);
        assert_eq!(properties[field]["items"]["minimum"], i32::MIN);
        assert_eq!(properties[field]["items"]["maximum"], i32::MAX);
    }

    let decoded = adapter.decode(&goal_value()).unwrap();
    let expected = YardInput {
        buttons: 1u32 << 31,
        _pad: 0,
        origin: [32768, 0, -32768],
        dir: [0; 3],
    };
    assert_eq!(decoded, expected);
    assert_eq!(bytemuck::bytes_of(&decoded), bytemuck::bytes_of(&expected));
    assert!((adapter.commands)(PlayerSlot(0), &decoded).is_empty());
}

#[test]
fn mesh_goal_adapter_rejects_invalid_structured_values_before_lease_updates() {
    let adapter = input_adapter();
    for invalid in [
        json!({"buttons": 1u32 << 31, "origin": [0, 0], "dir": [0, 0, 0]}),
        json!({"buttons": 1u32 << 31, "origin": [0, 0, 0], "dir": [0, 0, 0], "_pad": 1}),
        json!({"buttons": 4294967296u64, "origin": [0, 0, 0], "dir": [0, 0, 0]}),
        json!({"buttons": 1u32 << 31, "origin": [2147483648i64, 0, 0], "dir": [0, 0, 0]}),
    ] {
        assert!(adapter.decode(&invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn configured_mesh_host_exposes_managed_goal_input_with_the_yard_player_limit() {
    let fixture = Fixture::new();
    let mut config = ServerConfig::new(Auth::DevNoAuth);
    config.listen = false;
    let host = spawn_mesh_navigation_scene_host(fixture.scene.clone(), config).unwrap();
    let mut client = client(&host);

    let discovery = client.call("rpc.discover", json!({})).unwrap();
    let descriptor = client.call("registry.input", json!({})).unwrap();
    assert_eq!(discovery["engine"]["game"], "MeshNavigation");
    assert_eq!(descriptor["schema"]["title"], "MeshGoalInput");
    assert_eq!(descriptor["max_players"], YARD_PLAYERS);
    assert_eq!(descriptor["managed_held"]["version"], 1);
    assert_eq!(discovery["engine"]["input"], descriptor);
    assert!(client
        .call("sim.start", json!({"player_count": YARD_PLAYERS + 1}))
        .is_err());
    client.call("sim.start", json!({})).unwrap();

    let grant = client
        .call(
            "sim.input_claim",
            json!({"player": 0, "replace_held": true}),
        )
        .unwrap();
    let tagged = |sequence: &str, value: Value| {
        json!({
            "player": 0,
            "grant": grant["grant"],
            "generation": grant["generation"],
            "sequence": sequence,
            "value": value
        })
    };
    assert!(client
        .call(
            "sim.input_value",
            tagged("1", {
                let mut invalid = goal_value();
                invalid["_pad"] = json!(1);
                invalid
            })
        )
        .is_err());
    client
        .call("sim.input_value", tagged("1", goal_value()))
        .unwrap();
}
