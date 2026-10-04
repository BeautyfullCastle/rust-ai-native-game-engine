//! ERP integration coverage for one atomic, bounded adapter-position batch.
mod common;

use common::TestHost;
use orr_edit::EditorDoc;
use orr_reflect::TypeRegistry;
use orr_remote::{Auth, Caps, ErpClient, GameHooks, LocalHost, RpcError, ServerConfig};
use orr_sim::Simulation;
use orr_testgame::Arena;
use serde_json::{json, Value as J};

const COMPONENT: &str = "orr_physics::Body";
const ARENA_SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [-10,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    Position: { pos: [10,0] }\n    PlayerTag: { slot: 1 }\n";

fn two_bodies(c: &mut ErpClient) -> (String, String) {
    let result = c
        .call(
            "world.query",
            json!({"components": [COMPONENT], "limit": 2}),
        )
        .unwrap();
    let entities = result["entities"].as_array().unwrap();
    assert_eq!(
        entities.len(),
        2,
        "fixture should expose two physics bodies: {result}"
    );
    (
        entities[0]["guid"].as_str().unwrap().to_string(),
        entities[1]["guid"].as_str().unwrap().to_string(),
    )
}

fn doc_checksum(c: &mut ErpClient) -> String {
    c.call("sim.state", J::Null).unwrap()["doc_checksum"]
        .as_str()
        .unwrap()
        .to_string()
}

fn stale_checksum(current: &str) -> String {
    let mut stale = current.to_string();
    let last = stale.pop().expect("canonical checksum is nonempty");
    stale.push(if last == '0' { '1' } else { '0' });
    stale
}

fn history(c: &mut ErpClient) -> J {
    c.call("history.list", J::Null).unwrap()
}

fn position(c: &mut ErpClient, guid: &str) -> J {
    c.call(
        "world.get",
        json!({"entity": guid, "component": COMPONENT, "path": "pos"}),
    )
    .unwrap()["value"]
        .clone()
}

fn batch(checksum: &str, patches: J) -> J {
    json!({
        "label": "Move selected entities",
        "expected_checksum": checksum,
        "component": COMPONENT,
        "path": "pos",
        "patches": patches,
    })
}

fn error_kind(error: RpcError) -> String {
    error.kind().unwrap_or("missing_kind").to_string()
}

#[test]
fn patch_batch_is_one_undoable_edit_and_noops_leave_history_unchanged() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let discovered = c.call("rpc.discover", J::Null).unwrap();
    assert!(discovered["methods"]
        .as_array()
        .unwrap()
        .iter()
        .any(|method| {
            method["name"] == "world.patch_batch"
                && method["capability"] == "scene_edit"
                && method["allowed"].as_bool() == Some(true)
        }));

    let (first, second) = two_bodies(&mut c);
    let before = history(&mut c);
    let original_first = position(&mut c, &first);
    let original_second = position(&mut c, &second);
    let checksum = doc_checksum(&mut c);
    let result = c
        .call(
            "world.patch_batch",
            batch(
                &checksum,
                json!([
                    {"guid": first, "value": [101, 202]},
                    {"guid": second, "value": [-303, 404]},
                ]),
            ),
        )
        .unwrap();
    assert!(result["changed"].as_bool().unwrap());
    assert_eq!(result["count"], 2);
    assert_eq!(position(&mut c, &first), json!([101, 202]));
    assert_eq!(position(&mut c, &second), json!([-303, 404]));

    let after = history(&mut c);
    assert_eq!(
        after["entries"].as_array().unwrap().len(),
        before["entries"].as_array().unwrap().len() + 1,
        "a successful multi-entity RPC is one history entry"
    );
    let entry = after["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(entry["label"], "Move selected entities");
    assert_eq!(entry["origin"], "agent:claude");
    assert_eq!(entry["op_count"], 2);

    c.call("history.undo", J::Null).unwrap();
    assert_eq!(position(&mut c, &first), original_first);
    assert_eq!(position(&mut c, &second), original_second);
    assert_eq!(doc_checksum(&mut c), checksum);
    let undone = history(&mut c);
    assert_eq!(undone["entries"].as_array().unwrap().len(), 1);
    assert_eq!(undone["can_redo"], true);
    let undone_entry = undone["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(undone_entry["label"], entry["label"]);
    assert_eq!(undone_entry["origin"], entry["origin"]);
    assert_eq!(undone_entry["op_count"], entry["op_count"]);
    assert_eq!(undone_entry["undone"], true);
    c.call("history.redo", J::Null).unwrap();
    assert_eq!(position(&mut c, &first), json!([101, 202]));
    assert_eq!(position(&mut c, &second), json!([-303, 404]));

    let before_noop = history(&mut c);
    let checksum = doc_checksum(&mut c);
    let noop = c
        .call(
            "world.patch_batch",
            batch(
                &checksum,
                json!([
                    {"guid": first, "value": [101, 202]},
                    {"guid": second, "value": [-303, 404]},
                ]),
            ),
        )
        .unwrap();
    assert!(!noop["changed"].as_bool().unwrap());
    assert_eq!(noop["count"], 2);
    assert_eq!(history(&mut c), before_noop);
}

#[test]
fn patch_batch_uses_the_arena_adapters_position_descriptor() {
    let host = LocalHost::spawn::<Arena>(|| {
        let mut types = TypeRegistry::new();
        orr_testgame::register_reflect(&mut types);
        let doc = EditorDoc::from_yaml(
            ARENA_SCENE,
            types,
            Simulation::<Arena>::build_registry(),
            42,
        )
        .map_err(|error| error.to_string())?;
        let mut config = ServerConfig::new(Auth::DevNoAuth);
        config.limits.game = GameHooks::new("Arena");
        Ok((doc, config))
    })
    .unwrap();
    let mut c = ErpClient::with_transport(Box::new(
        host.connector().connect("editor", Caps::ALL).unwrap(),
    ));
    let checksum = doc_checksum(&mut c);
    let result = c
        .call(
            "world.patch_batch",
            json!({
                "label": "Move Arena players",
                "expected_checksum": checksum,
                "component": "Position",
                "path": "pos",
                "patches": [
                    {"guid": "e_00000001", "value": [-20, 5]},
                    {"guid": "e_00000002", "value": [20, -5]},
                ],
            }),
        )
        .unwrap();
    assert!(result["changed"].as_bool().unwrap());
    assert_eq!(result["count"], 2);
    assert_eq!(
        c.call(
            "world.get",
            json!({"entity":"e_00000001","component":"Position","path":"pos"}),
        )
        .unwrap()["value"],
        json!([-20, 5])
    );
}

#[test]
fn invalid_suffix_preserves_scene_and_full_redo_history() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let (first, second) = two_bodies(&mut c);

    // Leave a real undone edit so redo is nonempty before the rejected batch.
    c.call(
        "world.patch",
        json!({"entity": first, "component": COMPONENT, "path": "pos", "value": [31, 41]}),
    )
    .unwrap();
    c.call("history.undo", J::Null).unwrap();
    let original_history = history(&mut c);
    assert_eq!(original_history["can_redo"], true);
    let original_checksum = doc_checksum(&mut c);
    let original_first = position(&mut c, &first);
    let original_second = position(&mut c, &second);

    let error = c.call_err(
        "world.patch_batch",
        batch(
            &original_checksum,
            json!([
                {"guid": first, "value": [500, 600]},
                {"guid": second, "value": [30001, 0]},
            ]),
        ),
    );
    assert_eq!(error.kind(), Some("invalid_value"), "{error}");
    assert_eq!(doc_checksum(&mut c), original_checksum);
    assert_eq!(position(&mut c, &first), original_first);
    assert_eq!(position(&mut c, &second), original_second);
    assert_eq!(history(&mut c), original_history);

    // The old redo entry remains intact and still reapplies successfully.
    c.call("history.redo", J::Null).unwrap();
    assert_eq!(position(&mut c, &first), json!([31, 41]));
}

#[test]
fn batch_rejects_stale_duplicate_missing_oversized_and_forbidden_requests() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let (first, second) = two_bodies(&mut c);
    let current = doc_checksum(&mut c);

    let stale = batch(
        &stale_checksum(&current),
        json!([{"guid": first, "value": [7, 8]}]),
    );
    assert_eq!(
        error_kind(c.call_err("world.patch_batch", stale)),
        "stale_checksum"
    );
    let dup = c.call_err(
        "world.patch_batch",
        batch(
            &current,
            json!([
                {"guid": first, "value": [7, 8]},
                {"guid": first, "value": [9, 10]},
            ]),
        ),
    );
    assert_eq!(dup.kind(), Some("invalid_params"));
    let missing = c.call_err(
        "world.patch_batch",
        batch(&current, json!([{"guid": "e_ffffffff", "value": [1, 2]}])),
    );
    assert_eq!(missing.kind(), Some("unknown_entity"));

    let too_many = (0..129)
        .map(|_| json!({"guid": second, "value": [1, 2]}))
        .collect::<Vec<_>>();
    assert_eq!(
        error_kind(c.call_err("world.patch_batch", batch(&current, json!(too_many)))),
        "limit_exceeded"
    );
    let too_wide = json!({
        "label": "x".repeat(70 * 1024),
        "expected_checksum": current,
        "component": COMPONENT,
        "path": "pos",
        "patches": [{"guid": first, "value": [1, 2]}],
    });
    assert_eq!(
        error_kind(c.call_err("world.patch_batch", too_wide)),
        "limit_exceeded"
    );

    let wrong_field = c.call_err(
        "world.patch_batch",
        json!({
            "label": "wrong descriptor",
            "expected_checksum": current,
            "component": COMPONENT,
            "path": "vel",
            "patches": [{"guid": first, "value": [1, 2]}],
        }),
    );
    assert_eq!(wrong_field.kind(), Some("invalid_params"));
    let wrong_component = c.call_err(
        "world.patch_batch",
        json!({
            "label": "wrong game adapter",
            "expected_checksum": current,
            "component": "Position",
            "path": "pos",
            "patches": [{"guid": first, "value": [1, 2]}],
        }),
    );
    assert_eq!(wrong_component.kind(), Some("invalid_params"));

    let mut reader = host.client("tok-read");
    assert_eq!(
        reader
            .call_err(
                "world.patch_batch",
                batch(&current, json!([{"guid": first, "value": [1, 2]}])),
            )
            .kind(),
        Some("permission_denied")
    );

    c.call("tx.begin", json!({"label": "held"})).unwrap();
    assert_eq!(
        c.call_err(
            "world.patch_batch",
            batch(&current, json!([{"guid": first, "value": [1, 2]}])),
        )
        .kind(),
        Some("tx_open")
    );
    c.call("tx.rollback", J::Null).unwrap();

    let mut other = host.client("tok-all");
    c.call("tx.begin", json!({"label": "foreign held"}))
        .unwrap();
    assert_eq!(
        other
            .call_err(
                "world.patch_batch",
                batch(&current, json!([{"guid": first, "value": [1, 2]}])),
            )
            .kind(),
        Some("tx_busy")
    );
    c.call("tx.rollback", J::Null).unwrap();

    c.call("sim.start", J::Null).unwrap();
    assert_eq!(
        c.call_err(
            "world.patch_batch",
            batch(&current, json!([{"guid": first, "value": [1, 2]}])),
        )
        .kind(),
        Some("sim_running")
    );
    c.call("sim.stop", J::Null).unwrap();
}
