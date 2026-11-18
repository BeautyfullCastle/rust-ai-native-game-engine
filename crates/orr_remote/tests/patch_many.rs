//! ERP coverage for `world.patch_many`: one field of one component type on many
//! entities, as one undo step.

use orr_edit::EditorDoc;
use orr_reflect::TypeRegistry;
use orr_remote::{Auth, Caps, ErpClient, GameHooks, LocalHost, ServerConfig};
use orr_sim::Simulation;
use orr_testgame::Arena;
use serde_json::{json, Value as J};

const COUNT: usize = 200;

fn scene() -> String {
    let mut text = String::from("schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n");
    for i in 1..=COUNT {
        text.push_str(&format!("  e_{i:08x}:\n    Position: {{ pos: [0,0] }}\n"));
    }
    text
}

fn host() -> LocalHost {
    LocalHost::spawn::<Arena>(|| {
        let mut types = TypeRegistry::new();
        orr_testgame::register_reflect(&mut types);
        let doc = EditorDoc::from_yaml(&scene(), types, Simulation::<Arena>::build_registry(), 42).map_err(|error| error.to_string())?;
        let mut config = ServerConfig::new(Auth::DevNoAuth);
        config.limits.game = GameHooks::new("Arena");
        Ok((doc, config))
    })
    .unwrap()
}

fn client(host: &LocalHost) -> ErpClient {
    ErpClient::with_transport(Box::new(host.connector().connect("editor", Caps::ALL).unwrap()))
}

fn guid(i: usize) -> String {
    format!("e_{i:08x}")
}

fn position(c: &mut ErpClient, i: usize) -> J {
    c.call("world.get", json!({"entity": guid(i), "component": "Position", "path": "pos"})).unwrap()["value"].clone()
}

fn doc_checksum(c: &mut ErpClient) -> String {
    c.call("sim.state", J::Null).unwrap()["doc_checksum"].as_str().unwrap().to_string()
}

fn entries(c: &mut ErpClient) -> usize {
    c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len()
}

fn patches(from: usize, to: usize, value: impl Fn(usize) -> J) -> J {
    J::Array((from..=to).map(|i| json!({"entity": guid(i), "value": value(i)})).collect())
}

fn request(patches: J) -> J {
    json!({"label": "Move many", "component": "Position", "path": "pos", "patches": patches})
}

#[test]
fn one_request_sets_the_field_of_many_entities_as_one_undo_step() {
    let host = host();
    let mut c = client(&host);
    assert!(c.call("rpc.discover", J::Null).unwrap()["methods"].as_array().unwrap().iter().any(|m| m["name"] == "world.patch_many" && m["capability"] == "scene_edit"));
    let before = doc_checksum(&mut c);
    let history = entries(&mut c);

    let result = c.call("world.patch_many", request(patches(1, COUNT, |i| json!([i, -(i as i64)])))).unwrap();
    assert!(result["changed"].as_bool().unwrap());
    assert_eq!(result["count"], COUNT);
    assert_eq!(position(&mut c, 1), json!([1, -1]));
    assert_eq!(position(&mut c, COUNT), json!([COUNT, -(COUNT as i64)]));
    assert_eq!(entries(&mut c), history + 1, "{COUNT} entities are one history entry");
    let log = c.call("history.list", J::Null).unwrap();
    let entry = log["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(entry["label"], "Move many");
    assert_eq!(entry["op_count"], COUNT);

    c.call("history.undo", J::Null).unwrap();
    assert_eq!(doc_checksum(&mut c), before, "one undo reverses every entity");
    assert_eq!(position(&mut c, 77), json!([0, 0]));
    c.call("history.redo", J::Null).unwrap();
    assert_eq!(position(&mut c, 77), json!([77, -77]));

    // The same values again change nothing and add no history entry.
    let history = entries(&mut c);
    let again = c.call("world.patch_many", request(patches(1, COUNT, |i| json!([i, -(i as i64)])))).unwrap();
    assert!(!again["changed"].as_bool().unwrap());
    assert_eq!(entries(&mut c), history);
}

#[test]
fn a_refused_request_changes_nothing() {
    let host = host();
    let mut c = client(&host);
    let before = doc_checksum(&mut c);
    let history = entries(&mut c);
    let bad: Vec<(&str, J)> = vec![
        ("unknown entity", request(json!([{"entity": guid(1), "value": [5, 5]}, {"entity": "e_0000ffff", "value": [6, 6]}]))),
        ("a value of the wrong type", request(json!([{"entity": guid(1), "value": [5, 5]}, {"entity": guid(2), "value": "far"}]))),
        ("a repeated entity", request(json!([{"entity": guid(1), "value": [5, 5]}, {"entity": guid(1), "value": [6, 6]}]))),
        ("no patches", request(json!([]))),
        ("a stray key", request(json!([{"entity": guid(1), "value": [5, 5], "extra": 1}]))),
        ("an unknown component", json!({"component": "Nope", "path": "pos", "patches": [{"entity": guid(1), "value": [5, 5]}]})),
        ("an empty label", json!({"label": " ", "component": "Position", "path": "pos", "patches": [{"entity": guid(1), "value": [5, 5]}]})),
    ];
    for (what, params) in bad {
        let error = c.call_err("world.patch_many", params);
        assert!(error.kind().is_some(), "{what}: {error}");
        assert_eq!(doc_checksum(&mut c), before, "{what} left the scene changed");
        assert_eq!(entries(&mut c), history, "{what} left a history entry");
    }
    // The limit is on the number of entities in a request.
    let too_many = request(J::Array((0..4097).map(|_| json!({"entity": guid(1), "value": [1, 1]})).collect()));
    assert_eq!(c.call_err("world.patch_many", too_many).kind(), Some("limit_exceeded"));
    assert_eq!(doc_checksum(&mut c), before);
}

#[test]
fn inside_a_transaction_the_edits_join_it_and_the_commit_is_one_undo_step() {
    let host = host();
    let mut c = client(&host);
    let before = doc_checksum(&mut c);
    let history = entries(&mut c);
    c.call("tx.begin", json!({"label": "drag"})).unwrap();
    c.call("world.patch_many", request(patches(1, COUNT, |i| json!([i, 1])))).unwrap();
    c.call("world.patch_many", request(patches(1, COUNT, |i| json!([i, 2])))).unwrap();
    c.call("tx.commit", J::Null).unwrap();
    assert_eq!(position(&mut c, 9), json!([9, 2]));
    assert_eq!(entries(&mut c), history + 1);
    c.call("history.undo", J::Null).unwrap();
    assert_eq!(doc_checksum(&mut c), before);

    // A rollback takes the whole batch back.
    c.call("tx.begin", json!({"label": "drag"})).unwrap();
    c.call("world.patch_many", request(patches(1, COUNT, |i| json!([i, 3])))).unwrap();
    c.call("tx.rollback", J::Null).unwrap();
    assert_eq!(doc_checksum(&mut c), before);
}
