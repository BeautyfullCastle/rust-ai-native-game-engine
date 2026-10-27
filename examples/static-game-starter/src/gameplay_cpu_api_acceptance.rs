//! In-process ERP acceptance. Project bootstrap is a local SDK operation;
//! cross-game project opening over a live ERP connection is not implemented.
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
fn control(editor: &mut Editor, api: &mut Api, fields: J) -> J {
    let description = describe(editor, api);
    let request = api.mutation(&description, fields);
    let response = call(editor, api, "editor.gameplay.control", request).unwrap();
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
