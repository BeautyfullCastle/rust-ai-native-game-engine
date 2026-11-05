//! In-process ERP acceptance. Project bootstrap is a local SDK operation;
//! cross-game project opening over a live ERP connection is not implemented.
#![allow(
    clippy::disallowed_types,
    reason = "Wall-clock deadlines bound CPU ERP acceptance tests, not simulation"
)]
use std::time::{Duration, Instant};

use orr_editor::{registered_project::PreparedRegisteredProject, Editor};
use orr_remote::{Caps, ErpClient, RpcError, METHOD_NOT_FOUND, PERMISSION_DENIED};
use serde_json::{json, Value as J};

struct Api {
    client: ErpClient,
    next_key: u64,
}
impl Api {
    fn new(editor: &Editor, name: &str, capabilities: &str) -> Self {
        Self {
            client: editor
                .gameplay_api_client(name, Caps::parse(capabilities).unwrap())
                .unwrap(),
            next_key: 0,
        }
    }
    fn mutation(&mut self, description: &J, fields: J) -> J {
        self.next_key += 1;
        let mut request = json!({
            "binding":description["binding"],
            "connection":description["connection"],
            "scope":description["scope"],
            "request_key":self.next_key.to_string(),
            "timeout_ms":5000,
        });
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        request
    }
}

// The SDK owns the local editor event loop, just as eframe does in production.
// Every tested operation below is an actual ERP request over its own transport.
fn call(editor: &mut Editor, api: &mut Api, method: &str, params: J) -> Result<J, RpcError> {
    let id = api.client.post(method, params).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        editor.pump();
        api.client.poll().unwrap();
        if let Some(index) = api
            .client
            .responses
            .iter()
            .position(|(response_id, _)| *response_id == id)
        {
            return api.client.responses.remove(index).unwrap().1;
        }
        assert!(Instant::now() < deadline, "ERP response deadline: {method}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn describe(editor: &mut Editor, api: &mut Api) -> J {
    let value = call(editor, api, "editor.gameplay.describe", json!({})).unwrap();
    for field in ["binding", "connection", "scope"] {
        assert!(value[field].as_str().is_some(), "missing gameplay {field}");
    }
    value
}
#[track_caller]
fn control(editor: &mut Editor, api: &mut Api, fields: J) -> J {
    let stage = std::panic::Location::caller();
    let operation = fields.to_string();
    let description = describe(editor, api);
    let request = api.mutation(&description, fields);
    let response = call(editor, api, "editor.gameplay.control", request).unwrap_or_else(|error| {
        panic!("ERP control {operation} at {stage} failed after describe={description}: {error:?}")
    });
    assert_eq!(response["status"], "queued");
    response
}
fn wait_state(editor: &mut Editor, api: &mut Api, done: impl Fn(&J) -> bool) -> J {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = call(editor, api, "sim.state", J::Null).unwrap();
        if done(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "ERP state progress deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn wait_ready(editor: &mut Editor, api: &mut Api, field: &str) -> J {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let description = describe(editor, api);
        assert_eq!(description["error"], false);
        if description[field] == true {
            return description;
        }
        assert!(Instant::now() < deadline, "ERP command readiness deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn status(editor: &mut Editor, api: &mut Api, command: &J) -> Result<J, RpcError> {
    let description = describe(editor, api);
    call(
        editor,
        api,
        "editor.gameplay.status",
        json!({"binding":description["binding"],"connection":description["connection"],"command_id":command}),
    )
}
fn fixture(alias: &str, scene: &str) -> (tempfile::TempDir, std::path::PathBuf, Editor) {
    let (manifest, scene_name) = match alias {
        "delivery" => (
            include_str!("../scenes/delivery.project.json"),
            "delivery.scene.yaml",
        ),
        "switchboard" => (
            include_str!("../scenes/switchboard.project.json"),
            "switchboard.scene.yaml",
        ),
        _ => panic!("unknown fixture"),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, manifest).unwrap();
    let scene_path = dir.path().join(scene_name);
    std::fs::write(&scene_path, scene).unwrap();
    // Bootstrap is deliberately local and root-bounded. It does not claim a
    // network project-open RPC or a runtime cross-game typed-host replacement.
    let prepared = PreparedRegisteredProject::open(&path, super::registry()).unwrap();
    let mut editor = Editor::open_registered_project(prepared).unwrap();
    editor.sync();
    (dir, scene_path, editor)
}

#[test]
fn two_saved_games_support_api_only_authoring_commands_and_reconnect_fences() {
    for (alias, scene, actor, singleton, field, edited_value) in [
        (
            "delivery",
            crate::delivery::SCENE,
            "Courier",
            "Delivery",
            "goal",
            json!([0, 0]),
        ),
        (
            "switchboard",
            crate::switchboard::SCENE,
            "Cursor",
            "Board",
            "target",
            json!(1),
        ),
    ] {
        let (_dir, scene_path, mut editor) = fixture(alias, scene);
        let mut writer = Api::new(&editor, "api-writer", "read,scene_edit,sim_control");
        let mut reader = Api::new(&editor, "api-reader", "read");
        let mut other = Api::new(&editor, "api-other", "read,scene_edit,sim_control");
        let discovery = call(&mut editor, &mut reader, "rpc.discover", J::Null).unwrap();
        let methods = discovery["methods"].as_array().unwrap();
        for method in [
            "editor.gameplay.describe",
            "editor.gameplay.control",
            "editor.gameplay.submit",
            "editor.gameplay.status",
            "editor.gameplay.cancel",
        ] {
            assert!(methods.iter().any(|entry| entry["name"] == method));
        }
        assert_eq!(
            call(&mut editor, &mut writer, "editor.project.open", json!({}))
                .unwrap_err()
                .code,
            METHOD_NOT_FOUND,
            "cross-game project opening is not silently emulated with scene.load"
        );
        let original = call(&mut editor, &mut reader, "scene.save", J::Null).unwrap();
        let query = call(
            &mut editor,
            &mut reader,
            "world.query",
            json!({"components":[actor],"values":true,"limit":8}),
        )
        .unwrap();
        assert_eq!(query["total"], 1);
        let guid = query["entities"][0]["guid"].as_str().unwrap().to_owned();
        assert_eq!(guid, "e_00000001");
        let position = call(
            &mut editor,
            &mut reader,
            "world.get",
            json!({"entity":guid,"component":actor,"path":"position"}),
        )
        .unwrap();
        assert_eq!(position["value"], json!([0, 0]));

        let patch = json!({"name":singleton,"path":field,"value":edited_value});
        assert_eq!(
            call(
                &mut editor,
                &mut reader,
                "world.singleton.patch",
                patch.clone()
            )
            .unwrap_err()
            .code,
            PERMISSION_DENIED
        );
        for method in ["editor.gameplay.control", "editor.gameplay.submit"] {
            let description = describe(&mut editor, &mut reader);
            let fields = if method.ends_with("control") {
                json!({"action":"play"})
            } else {
                json!({"intent":"act","payload":{"press":1}})
            };
            let request = reader.mutation(&description, fields);
            assert_eq!(
                call(&mut editor, &mut reader, method, request)
                    .unwrap_err()
                    .code,
                PERMISSION_DENIED
            );
        }
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            original
        );
        let before_authoring = describe(&mut editor, &mut writer);
        call(&mut editor, &mut writer, "world.singleton.patch", patch).unwrap();
        let edited = call(&mut editor, &mut reader, "scene.save", J::Null).unwrap();
        assert_ne!(edited["checksum"], original["checksum"]);
        call(&mut editor, &mut writer, "history.undo", J::Null).unwrap();
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            original
        );
        call(&mut editor, &mut writer, "history.redo", J::Null).unwrap();
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            edited
        );
        let history = call(&mut editor, &mut reader, "history.list", J::Null).unwrap();

        // Host authoring ACKs precede the paired editor's notification/cache
        // refresh. Settle that projection before asking for a gameplay scope;
        // otherwise redo may invalidate describe's scope before Play arrives.
        // This SDK observation barrier neither edits the host nor retries a
        // fenced mutation. The expected document is still read through ERP.
        editor.sync();
        assert!(editor.down().is_none());
        let edited_checksum = orr_remote::wire::parse_checksum(&edited["checksum"]).unwrap();
        assert_eq!(editor.sim().checksum, edited_checksum);
        assert_eq!(editor.checksum(), edited_checksum);
        assert_eq!(editor.mode(), orr_editor::Mode::Edit);
        // Explicitly prove the old authoring cut is rejected, then make the
        // intended Play request once with a newly described, settled scope.
        let stale_play = writer.mutation(&before_authoring, json!({"action":"play"}));
        let stale = call(
            &mut editor,
            &mut writer,
            "editor.gameplay.control",
            stale_play,
        )
        .unwrap_err();
        assert_eq!(stale.kind(), Some("editor_gameplay_rejected"));
        assert_eq!(stale.data.as_ref().unwrap()["reason"], "stale_scope");
        assert_eq!(
            call(&mut editor, &mut reader, "sim.state", J::Null).unwrap()["mode"],
            "edit"
        );
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            edited
        );
        control(&mut editor, &mut writer, json!({"action":"play"}));
        wait_state(&mut editor, &mut writer, |state| state["playing"] == true);
        wait_ready(&mut editor, &mut writer, "can_claim");
        control(
            &mut editor,
            &mut writer,
            json!({"action":"claim","player":0}),
        );
        let description = wait_ready(&mut editor, &mut writer, "can_submit");
        let secret = "api-invalid-payload-must-not-appear-in-activity";
        let malformed = writer.mutation(
            &description,
            json!({"intent":"act","payload":{"press":1,"secret":secret}}),
        );
        assert!(call(
            &mut editor,
            &mut writer,
            "editor.gameplay.submit",
            malformed
        )
        .is_err());
        let description = wait_ready(&mut editor, &mut writer, "can_submit");
        let submit = writer.mutation(&description, json!({"intent":"act","payload":{"press":1}}));
        let accepted = call(
            &mut editor,
            &mut writer,
            "editor.gameplay.submit",
            submit.clone(),
        )
        .unwrap();
        let command = accepted["command_id"].clone();
        assert!(command.as_str().unwrap().parse::<u64>().unwrap() > 0);
        // Exact retries refer to one admitted command, even if its ACK arrived
        // before the frame which proves consumption. They never submit again.
        let retried = call(
            &mut editor,
            &mut writer,
            "editor.gameplay.submit",
            submit.clone(),
        )
        .unwrap();
        assert_eq!(retried["command_id"], command);
        let mut conflict = submit;
        conflict["payload"] = json!({"press":0});
        let conflict =
            call(&mut editor, &mut writer, "editor.gameplay.submit", conflict).unwrap_err();
        assert_eq!(conflict.kind(), Some("editor_gameplay_key_conflict"));

        assert_eq!(
            status(&mut editor, &mut other, &command).unwrap_err().code,
            PERMISSION_DENIED
        );
        let description = describe(&mut editor, &mut other);
        let cancel = other.mutation(&description, json!({"command_id":command}));
        assert_eq!(
            call(&mut editor, &mut other, "editor.gameplay.cancel", cancel)
                .unwrap_err()
                .code,
            PERMISSION_DENIED
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let completed = loop {
            let result = status(&mut editor, &mut writer, &command).unwrap();
            if result["status"] == "completed" {
                break result;
            }
            assert!(
                result["status"] == "pending" || result["status"] == "consumed",
                "command failed before consumption"
            );
            assert!(Instant::now() < deadline, "ERP consumption deadline");
            std::thread::sleep(Duration::from_millis(1));
        };
        let consumed = completed["consumed"]["tick"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let neutral = completed["neutral"]["tick"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!(neutral > consumed);
        for field in ["stream_generation", "play_epoch", "timeline_epoch"] {
            assert_eq!(completed["consumed"][field], completed["neutral"][field]);
        }
        let game_state = call(
            &mut editor,
            &mut reader,
            "world.singleton.get",
            json!({"name":singleton}),
        )
        .unwrap();
        if alias == "delivery" {
            assert_eq!(game_state["value"]["won"], 1);
            assert_eq!(game_state["value"]["action_held"], 0);
        } else {
            assert_eq!(game_state["value"]["solved"], 1);
            assert_eq!(game_state["value"]["moves"], 1);
        }
        let activity = call(
            &mut editor,
            &mut reader,
            "activity.list",
            json!({"limit":128,"include_reads":false}),
        )
        .unwrap();
        let command_token = format!("command={}", command.as_str().unwrap());
        let actual_values = activity["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| {
                entry["method"] == "sim.input_value"
                    && entry["ok"] == true
                    && entry["summary"].as_str().is_some_and(|summary| {
                        summary
                            .split_whitespace()
                            .any(|token| token == command_token)
                    })
            })
            .count();
        assert_eq!(
            actual_values, 1,
            "same-key retries must not reach managed input twice"
        );
        assert!(!activity.to_string().contains(secret));

        control(&mut editor, &mut writer, json!({"action":"pause"}));
        let paused = wait_state(&mut editor, &mut writer, |state| state["playing"] == false);
        let paused_tick = paused["head_tick"].as_u64().unwrap();
        for _ in 0..5 {
            assert_eq!(
                call(&mut editor, &mut reader, "sim.state", J::Null).unwrap()["head_tick"],
                paused_tick
            );
        }
        control(&mut editor, &mut writer, json!({"action":"step","n":1}));
        wait_state(&mut editor, &mut writer, |state| {
            state["head_tick"] == paused_tick + 1 && state["playing"] == false
        });
        let old_description = describe(&mut editor, &mut writer);
        let old_connection_request = writer.mutation(&old_description, json!({"action":"play"}));
        drop(writer);
        let mut writer = Api::new(
            &editor,
            "api-writer-reconnected",
            "read,scene_edit,sim_control",
        );
        let new_description = describe(&mut editor, &mut writer);
        assert_ne!(new_description["connection"], old_description["connection"]);
        let stale = call(
            &mut editor,
            &mut writer,
            "editor.gameplay.control",
            old_connection_request,
        )
        .unwrap_err();
        assert_eq!(stale.kind(), Some("editor_gameplay_stale"));
        assert_eq!(
            call(&mut editor, &mut reader, "sim.state", J::Null).unwrap()["playing"],
            false
        );
        control(&mut editor, &mut writer, json!({"action":"stop"}));
        wait_state(&mut editor, &mut writer, |state| state["mode"] == "edit");
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            edited
        );
        assert_eq!(
            call(&mut editor, &mut reader, "history.list", J::Null).unwrap(),
            history
        );
        call(&mut editor, &mut writer, "history.undo", J::Null).unwrap();
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            original
        );
        assert_eq!(std::fs::read_to_string(&scene_path).unwrap(), scene);
    }
}

fn stock_api_state(editor: &mut Editor, api: &mut Api, actor: &str, singleton: &str) -> J {
    let query = call(
        editor,
        api,
        "world.query",
        json!({"components":[actor],"values":true}),
    )
    .unwrap();
    assert_eq!(query["total"], 1);
    let entity = &query["entities"][0]["guid"];
    let actor = call(
        editor,
        api,
        "world.get",
        json!({"entity":entity,"component":actor}),
    )
    .unwrap();
    let singleton = call(
        editor,
        api,
        "world.singleton.get",
        json!({"name":singleton}),
    )
    .unwrap();
    json!({"actor":actor["value"],"singleton":singleton["value"]})
}

fn stock_api_command(editor: &mut Editor, api: &mut Api, intent: &str, payload: J) -> (J, J) {
    let description = wait_ready(editor, api, "can_submit");
    let request = api.mutation(&description, json!({"intent":intent,"payload":payload}));
    let accepted = call(editor, api, "editor.gameplay.submit", request.clone()).unwrap();
    let command = accepted["command_id"].clone();
    let retry = call(editor, api, "editor.gameplay.submit", request.clone()).unwrap();
    assert_eq!(retry["command_id"], command);
    let deadline = Instant::now() + Duration::from_secs(5);
    let completed = loop {
        let value = status(editor, api, &command).unwrap();
        if value["status"] == "completed" {
            break value;
        }
        assert!(
            value["status"] == "pending" || value["status"] == "consumed",
            "{value}"
        );
        assert!(
            Instant::now() < deadline,
            "semantic stock command did not complete: {intent}"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    let consumed = completed["consumed"]["tick"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let neutral = completed["neutral"]["tick"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(neutral > consumed);
    for key in ["stream_generation", "play_epoch", "timeline_epoch"] {
        assert_eq!(completed["consumed"][key], completed["neutral"][key]);
    }
    println!("stock semantic ERP intent={intent} payload={payload} result={completed}");
    (command, request)
}

#[test]
fn stock_registered_projects_complete_via_semantic_erp_and_do_not_replay() {
    for (alias, scene, actor, singleton, navigation) in [
        (
            "delivery",
            crate::delivery::SCENE,
            "Courier",
            "Delivery",
            "move",
        ),
        (
            "switchboard",
            crate::switchboard::SCENE,
            "Cursor",
            "Board",
            "select",
        ),
    ] {
        let (dir, scene_path, mut editor) = fixture(alias, scene);
        let manifest = std::fs::read(dir.path().join("project.json")).unwrap();
        let mut writer = Api::new(&editor, "stock-semantic-writer", "read,sim_control");
        let mut reader = Api::new(&editor, "stock-semantic-reader", "read");
        let original = call(&mut editor, &mut reader, "scene.save", J::Null).unwrap();
        let history = call(&mut editor, &mut reader, "history.list", J::Null).unwrap();
        let initial = stock_api_state(&mut editor, &mut reader, actor, singleton);
        let catalog = describe(&mut editor, &mut reader);
        assert!(catalog["intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == navigation));
        let schema = call(
            &mut editor,
            &mut reader,
            "editor.gameplay.describe",
            json!({"intent":navigation}),
        )
        .unwrap();
        assert_eq!(schema["payload_schema"]["additionalProperties"], false);
        if alias == "delivery" {
            assert_eq!(initial["singleton"]["goal"], json!([4, 0]));
            assert_eq!(schema["payload_schema"]["required"], json!(["dx", "dy"]));
        } else {
            assert_eq!(initial["singleton"]["target"], 5);
            assert_eq!(schema["payload_schema"]["required"], json!(["delta"]));
        }
        control(&mut editor, &mut writer, json!({"action":"play"}));
        wait_state(&mut editor, &mut writer, |state| state["playing"] == true);
        wait_ready(&mut editor, &mut writer, "can_claim");
        control(
            &mut editor,
            &mut writer,
            json!({"action":"claim","player":0}),
        );
        let mut commands = Vec::new();
        if alias == "delivery" {
            for x in 1..=4 {
                commands.push(stock_api_command(
                    &mut editor,
                    &mut writer,
                    "move",
                    json!({"dx":1,"dy":0}),
                ));
                let state = stock_api_state(&mut editor, &mut reader, actor, singleton);
                assert_eq!(state["actor"]["position"], json!([x, 0]));
                assert_eq!(state["singleton"]["won"], 0);
                assert_eq!(state["singleton"]["action_held"], 0);
            }
        } else {
            commands.push(stock_api_command(
                &mut editor,
                &mut writer,
                "act",
                json!({"press":1}),
            ));
            for selected in 1..=2 {
                commands.push(stock_api_command(
                    &mut editor,
                    &mut writer,
                    "select",
                    json!({"delta":1}),
                ));
                let state = stock_api_state(&mut editor, &mut reader, actor, singleton);
                assert_eq!(state["actor"]["selected"], selected);
                assert_eq!(state["actor"]["held"], 0);
                assert_eq!(state["singleton"]["bits"], 1);
                assert_eq!(state["singleton"]["moves"], 1);
                assert_eq!(state["singleton"]["solved"], 0);
            }
        }
        commands.push(stock_api_command(
            &mut editor,
            &mut writer,
            "act",
            json!({"press":1}),
        ));
        let victory = stock_api_state(&mut editor, &mut reader, actor, singleton);
        assert_eq!(
            victory["singleton"][if alias == "delivery" { "won" } else { "solved" }],
            1
        );
        if alias == "switchboard" {
            assert_eq!(victory["singleton"]["bits"], 5);
        }
        commands.push(stock_api_command(
            &mut editor,
            &mut writer,
            "act",
            json!({"press":1}),
        ));
        let restarted = stock_api_state(&mut editor, &mut reader, actor, singleton);
        if alias == "delivery" {
            assert_eq!(restarted, initial);
        } else {
            assert_eq!(restarted["actor"]["selected"], 2);
            assert_eq!(restarted["actor"]["held"], 0);
            assert_eq!(restarted["singleton"], initial["singleton"]);
        }
        // A completed cancel is observational. Releasing/reclaiming changes the
        // control fence, but exact old request keys still cannot execute again.
        let description = describe(&mut editor, &mut writer);
        let cancel = writer.mutation(&description, json!({"command_id":commands[0].0}));
        assert_eq!(
            call(&mut editor, &mut writer, "editor.gameplay.cancel", cancel).unwrap()["status"],
            "completed"
        );
        control(&mut editor, &mut writer, json!({"action":"release"}));
        wait_ready(&mut editor, &mut writer, "can_claim");
        control(
            &mut editor,
            &mut writer,
            json!({"action":"claim","player":0}),
        );
        wait_ready(&mut editor, &mut writer, "can_submit");
        for (id, request) in &commands {
            match call(
                &mut editor,
                &mut writer,
                "editor.gameplay.submit",
                request.clone(),
            ) {
                Ok(retry) => assert_eq!(&retry["command_id"], id),
                // The bounded service may have evicted an old result after
                // intervening describe/status reads. It must then fail closed,
                // never treat an old request key as a fresh submission.
                Err(error) => assert_eq!(error.kind(), Some("editor_gameplay_stale")),
            }
            assert_eq!(
                status(&mut editor, &mut writer, id).unwrap()["status"],
                "completed"
            );
        }
        let before = call(&mut editor, &mut reader, "sim.state", J::Null).unwrap()["head_tick"]
            .as_u64()
            .unwrap();
        wait_state(&mut editor, &mut reader, |state| {
            state["head_tick"].as_u64().unwrap() >= before + 3
        });
        assert_eq!(
            stock_api_state(&mut editor, &mut reader, actor, singleton),
            restarted
        );
        let activity = call(
            &mut editor,
            &mut reader,
            "activity.list",
            json!({"limit":512,"include_reads":false}),
        )
        .unwrap();
        for (id, _) in &commands {
            let token = format!("command={}", id.as_str().unwrap());
            let count = activity["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| {
                    entry["method"] == "sim.input_value"
                        && entry["ok"] == true
                        && entry["summary"].as_str().is_some_and(|summary| {
                            summary.split_whitespace().any(|word| word == token)
                        })
                })
                .count();
            assert_eq!(count, 1, "retry replayed semantic command {id}");
        }
        control(&mut editor, &mut writer, json!({"action":"pause"}));
        wait_state(&mut editor, &mut writer, |state| state["playing"] == false);
        let paused = describe(&mut editor, &mut writer);
        let request = writer.mutation(&paused, json!({"intent":navigation,"payload":if alias == "delivery" { json!({"dx":1,"dy":0}) } else { json!({"delta":1}) }}));
        assert!(call(&mut editor, &mut writer, "editor.gameplay.submit", request).is_err());
        control(&mut editor, &mut writer, json!({"action":"stop"}));
        wait_state(&mut editor, &mut writer, |state| state["mode"] == "edit");
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            original
        );
        assert_eq!(
            call(&mut editor, &mut reader, "history.list", J::Null).unwrap(),
            history
        );
        assert_eq!(std::fs::read_to_string(scene_path).unwrap(), scene);
        assert_eq!(
            std::fs::read(dir.path().join("project.json")).unwrap(),
            manifest
        );
        println!("stock semantic ERP {alias} victory/restart/reclaim/no-replay and source/history preservation passed");
    }
}
