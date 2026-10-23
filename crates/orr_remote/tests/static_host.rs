use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_edit::EditorDoc;
use orr_reflect::{Reflect, TypeRegistry};
use orr_remote::{Auth, Caps, ErpClient, ExpectedInputDescriptor, LocalHost, ServerConfig};
use orr_sim::{Game, Simulation, System};
use serde_json::{json, Value as J};

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq, Pod, Zeroable, Reflect)]
struct NonzeroInput {
    direction: i32,
}
impl Default for NonzeroInput {
    fn default() -> Self {
        Self { direction: 7 }
    }
}
struct NonzeroGame;
impl Game for NonzeroGame {
    type Input = NonzeroInput;
    type Command = orr_testgame::SpawnBulletCmd;
    type Event = u32;
    type Config = ();
    fn register(_: &mut ComponentRegistryBuilder) {}
    fn setup(_: &mut Frame, _: &()) {}
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![]
    }
}
fn host(neutral: J) -> Result<LocalHost, String> {
    LocalHost::spawn_with_input::<NonzeroGame>(
        || {
            let doc = EditorDoc::from_yaml(
                "schema: orr.scene/1\nentities: {}\n",
                TypeRegistry::new(),
                Simulation::<NonzeroGame>::build_registry(),
                42,
            )
            .map_err(|e| e.to_string())?;
            let mut cfg = ServerConfig::new(Auth::DevNoAuth);
            cfg.listen = false;
            Ok((doc, cfg))
        },
        "NonzeroInput",
        2,
        |_, _| vec![],
        neutral,
        None,
    )
}
#[test]
fn nonzero_default_is_semantic_neutral_and_adapter_is_ready_without_listener() {
    let h = host(json!({"direction": 7})).expect("nonzero default is neutral");
    assert!(h.url().is_none());
    let mut c =
        ErpClient::with_transport(Box::new(h.connector().connect("test", Caps::ALL).unwrap()));
    let actual = c.call("registry.input", J::Null).unwrap();
    let expected = ExpectedInputDescriptor::new(
        actual["schema"].clone(),
        orr_remote::methods::VALUE_FORMAT,
        2,
    )
    .unwrap();
    expected.validate(&actual).unwrap();
    c.call("sim.start", json!({"player_count": 1})).unwrap();
    c.call(
        "sim.input_claim",
        json!({"player": 0, "replace_held": true}),
    )
    .unwrap();
}
#[test]
fn zero_bytes_and_malformed_values_are_rejected_before_setup() {
    for value in [
        json!({"direction": 0}),
        json!({"direction": "bad"}),
        J::Null,
    ] {
        let result = LocalHost::spawn_with_input::<NonzeroGame>(
            || panic!("invalid neutral reached setup"),
            "NonzeroInput",
            2,
            |_, _| vec![],
            value,
            None,
        );
        assert!(result.is_err());
    }
    assert!(LocalHost::spawn_with_input::<NonzeroGame>(
        || panic!("invalid bound reached setup"),
        "NonzeroInput",
        0,
        |_, _| vec![],
        json!({"direction":7}),
        None
    )
    .is_err());
}
#[test]
fn expected_input_is_validated_and_owned() {
    assert!(ExpectedInputDescriptor::new(J::Null, orr_remote::methods::VALUE_FORMAT, 1).is_err());
    assert!(ExpectedInputDescriptor::new(json!({}), "unknown", 1).is_err());
    assert!(
        ExpectedInputDescriptor::new(json!({}), orr_remote::methods::VALUE_FORMAT, 17).is_err()
    );
    let expected = ExpectedInputDescriptor::new(
        json!({"title":"Input"}),
        orr_remote::methods::VALUE_FORMAT,
        2,
    )
    .unwrap();
    let actual = json!({"schema": expected.schema(), "value_format": expected.value_format(), "max_players": expected.max_players(), "managed_held":{"version":1,"lease_ms":2000,"heartbeat_ms":500}});
    expected.validate(&actual).unwrap();
    for key in ["schema", "value_format", "max_players", "managed_held"] {
        let mut changed = actual.clone();
        changed.as_object_mut().unwrap().remove(key);
        assert!(expected.validate(&changed).is_err(), "{key}");
    }
    for key in ["version", "lease_ms", "heartbeat_ms"] {
        let mut changed = actual.clone();
        changed["managed_held"][key] = json!(99);
        assert!(expected.validate(&changed).is_err(), "{key}");
    }
}

#[test]
fn invalid_reflected_input_names_fail_before_setup() {
    for name in ["", "with space", "a.b", "name", "schema", "entities"] {
        let result = LocalHost::spawn_with_input::<NonzeroGame>(
            || panic!("invalid name reached setup"),
            name,
            2,
            |_, _| vec![],
            json!({"direction":7}),
            None,
        );
        assert!(result.is_err(), "{name}");
    }
}
