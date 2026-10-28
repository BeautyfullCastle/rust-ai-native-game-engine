//! Localhost-only transport acceptance, not a network project-open protocol.
//! Fixture credentials are fabricated per test and never read from the environment.
use std::time::{Duration, Instant};

use orr_editor::{registered_project::PreparedRegisteredProject, Editor};
use orr_remote::{
    Auth, Caps, ErpClient, RpcError, ServerConfig, TokenEntry, WsTransport, PERMISSION_DENIED,
};
use serde_json::{json, Value as J};

struct Api {
    client: ErpClient,
    next_key: u64,
}
impl Api {
    fn connect(editor: &mut Editor, url: &str, token: &str) -> Self {
        let mut api = Self {
            client: ErpClient::with_transport(Box::new(WsTransport::connect(url).unwrap())),
            next_key: 0,
        };
        call(editor, &mut api, "auth", json!({"token":token})).unwrap();
        api
    }
    fn mutation(&mut self, description: &J, fields: J) -> J {
        self.next_key += 1;
        let mut request = json!({
            "binding":description["binding"], "connection":description["connection"],
            "scope":description["scope"], "request_key":self.next_key.to_string(),
            "timeout_ms":5000,
        });
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        request
    }
}

// Each mutation is posted once. Only status/readiness reads are repeated;
// a timeout or transport failure is a test failure, never a mutation retry.
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
        assert!(
            Instant::now() < deadline,
            "WebSocket ERP response deadline: {method}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn describe(editor: &mut Editor, api: &mut Api) -> J {
    let description = call(editor, api, "editor.gameplay.describe", json!({})).unwrap();
    for field in ["binding", "connection", "scope"] {
        assert!(
            description[field].as_str().is_some(),
            "missing gameplay {field}"
        );
    }
    description
}
fn wait_read(
    editor: &mut Editor,
    api: &mut Api,
    method: &str,
    params: J,
    done: impl Fn(&J) -> bool,
) -> J {
    assert!(matches!(
        method,
        "sim.state" | "editor.gameplay.describe" | "editor.gameplay.status"
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let value = call(editor, api, method, params.clone()).unwrap();
        if done(&value) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "WebSocket ERP progress deadline: {method}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn control(editor: &mut Editor, api: &mut Api, fields: J) {
    let description = describe(editor, api);
    let params = api.mutation(&description, fields);
    assert_eq!(
        call(editor, api, "editor.gameplay.control", params).unwrap()["status"],
        "queued"
    );
}

#[test]
fn two_registered_games_accept_authenticated_websocket_semantic_input() {
    for (manifest, scene_name, scene, singleton, outcome) in [
        (
            include_str!("../scenes/delivery.project.json"),
            "delivery.scene.yaml",
            crate::delivery::SCENE.replace("goal: [4, 0]", "goal: [0, 0]"),
            "Delivery",
            "won",
        ),
        (
            include_str!("../scenes/switchboard.project.json"),
            "switchboard.scene.yaml",
            crate::switchboard::SCENE.replace("target: 5", "target: 1"),
            "Board",
            "solved",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("project.json");
        let scene_path = dir.path().join(scene_name);
        std::fs::write(&manifest_path, manifest).unwrap();
        std::fs::write(&scene_path, &scene).unwrap();
        // Deliberately non-production test values, unique to this temporary fixture.
        // No credential file, environment lookup, persistent configuration or URL token.
        let suffix = dir.path().file_name().unwrap().to_string_lossy();
        let reader_token = format!("test-only-reader-{suffix}");
        let writer_token = format!("test-only-controller-{suffix}");
        let mut config = ServerConfig::new(Auth::Tokens(vec![
            TokenEntry {
                token: reader_token.clone(),
                client: "ws-test-reader".into(),
                caps: Caps::parse("read").unwrap(),
            },
            TokenEntry {
                token: writer_token.clone(),
                client: "ws-test-controller".into(),
                caps: Caps::parse("read,sim_control").unwrap(),
            },
        ]));
        config.bind = "127.0.0.1:0".parse().unwrap();
        assert!(config.bind.ip().is_loopback());
        assert_eq!(config.bind.port(), 0);
        // Project selection stays a trusted, root-bounded SDK bootstrap.
        let prepared = PreparedRegisteredProject::open(&manifest_path, super::registry()).unwrap();
        let spec = prepared.host_spec().with_listener(config);
        let mut editor = Editor::start(&spec).unwrap();
        editor.sync();
        let url = editor.erp_status().unwrap().0;
        let address: std::net::SocketAddr = url
            .strip_prefix("ws://")
            .unwrap()
            .trim_end_matches('/')
            .parse()
            .unwrap();
        assert!(address.ip().is_loopback());
        assert_ne!(address.port(), 0);
        let mut reader = Api::connect(&mut editor, &url, &reader_token);
        let mut writer = Api::connect(&mut editor, &url, &writer_token);
        let original = call(&mut editor, &mut reader, "scene.save", J::Null).unwrap();
        let history = call(&mut editor, &mut reader, "history.list", J::Null).unwrap();
        for (method, fields) in [
            ("editor.gameplay.control", json!({"action":"play"})),
            (
                "editor.gameplay.submit",
                json!({"intent":"act","payload":{"press":1}}),
            ),
        ] {
            let description = describe(&mut editor, &mut reader);
            let request = reader.mutation(&description, fields);
            assert_eq!(
                call(&mut editor, &mut reader, method, request)
                    .unwrap_err()
                    .code,
                PERMISSION_DENIED
            );
        }
        assert_eq!(
            call(&mut editor, &mut reader, "sim.state", J::Null).unwrap()["mode"],
            "edit"
        );
        control(&mut editor, &mut writer, json!({"action":"play"}));
        wait_read(&mut editor, &mut writer, "sim.state", J::Null, |v| {
            v["playing"] == true
        });
        wait_read(
            &mut editor,
            &mut writer,
            "editor.gameplay.describe",
            json!({}),
            |v| v["can_claim"] == true,
        );
        control(
            &mut editor,
            &mut writer,
            json!({"action":"claim","player":0}),
        );
        let description = wait_read(
            &mut editor,
            &mut writer,
            "editor.gameplay.describe",
            json!({}),
            |v| v["can_submit"] == true,
        );
        let request = writer.mutation(&description, json!({"intent":"act","payload":{"press":1}}));
        let accepted = call(&mut editor, &mut writer, "editor.gameplay.submit", request).unwrap();
        let command = &accepted["command_id"];
        assert!(command.as_str().unwrap().parse::<u64>().unwrap() > 0);
        let completed = wait_read(
            &mut editor,
            &mut writer,
            "editor.gameplay.status",
            json!({"binding":description["binding"],"connection":description["connection"],"command_id":command}),
            |v| {
                assert!(
                    matches!(
                        v["status"].as_str(),
                        Some("pending" | "consumed" | "completed")
                    ),
                    "unexpected command status"
                );
                v["status"] == "completed"
            },
        );
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
            assert!(completed["consumed"].get(field).is_some());
            assert_eq!(completed["consumed"][field], completed["neutral"][field]);
        }
        let state = call(
            &mut editor,
            &mut reader,
            "world.singleton.get",
            json!({"name":singleton}),
        )
        .unwrap();
        assert_eq!(state["value"][outcome], 1);
        if singleton == "Delivery" {
            assert_eq!(state["value"]["action_held"], 0);
        } else {
            assert_eq!(state["value"]["moves"], 1);
        }
        control(&mut editor, &mut writer, json!({"action":"stop"}));
        wait_read(&mut editor, &mut writer, "sim.state", J::Null, |v| {
            v["mode"] == "edit"
        });
        assert_eq!(
            call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
            original
        );
        assert_eq!(
            call(&mut editor, &mut reader, "history.list", J::Null).unwrap(),
            history
        );
        assert_eq!(std::fs::read_to_string(&scene_path).unwrap(), scene);
        assert_eq!(std::fs::read_to_string(&manifest_path).unwrap(), manifest);
        // Connections drop before Editor and its listener, then the temporary project.
    }
}

#[cfg(feature = "stormglass")]
#[test]
fn stormglass_authored_project_accepts_authenticated_websocket_summon_and_preserves_source() {
    use orr_games::coop_defense_game::{
        scenario_types, DefenseRun, DefenseUnit, SCENARIO_NAME, SUMMON,
    };

    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("project.json");
    let scene_path = dir.path().join("stormglass.scene.yaml");
    let manifest = include_str!("../scenes/stormglass.project.json");
    // Author the fixture before admission, not through an unfenced ERP mutation.
    // The authenticated socket exercises discovery, reads and gameplay only.
    let scene = "# authored WebSocket fixture\nschema: orr.scene/1\nsingletons:\n  CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 80}\nentities: {}\n";
    std::fs::write(&manifest_path, manifest).unwrap();
    std::fs::write(&scene_path, scene).unwrap();
    let suffix = dir.path().file_name().unwrap().to_string_lossy();
    let reader_token = format!("test-only-stormglass-reader-{suffix}");
    let writer_token = format!("test-only-stormglass-controller-{suffix}");
    let mut config = ServerConfig::new(Auth::Tokens(vec![
        TokenEntry {
            token: reader_token.clone(),
            client: "stormglass-ws-reader".into(),
            caps: Caps::parse("read").unwrap(),
        },
        TokenEntry {
            token: writer_token.clone(),
            client: "stormglass-ws-controller".into(),
            caps: Caps::parse("read,sim_control").unwrap(),
        },
    ]));
    config.bind = "127.0.0.1:0".parse().unwrap();
    assert!(config.bind.ip().is_loopback());
    assert_eq!(config.bind.port(), 0);
    let prepared =
        PreparedRegisteredProject::open(&manifest_path, crate::stormglass::registry()).unwrap();
    let spec = prepared.host_spec().with_listener(config);
    let mut editor = Editor::start(&spec).unwrap();
    editor.sync();
    let url = editor.erp_status().unwrap().0;
    let address: std::net::SocketAddr = url
        .strip_prefix("ws://")
        .unwrap()
        .trim_end_matches('/')
        .parse()
        .unwrap();
    assert!(address.ip().is_loopback());
    assert_ne!(address.port(), 0);
    let mut reader = Api::connect(&mut editor, &url, &reader_token);
    let mut writer = Api::connect(&mut editor, &url, &writer_token);
    let schema = call(
        &mut editor,
        &mut reader,
        "registry.schema",
        json!({"type":SCENARIO_NAME}),
    )
    .unwrap();
    assert_eq!(
        schema["schema"],
        serde_json::from_str::<J>(&scenario_types().type_json_schema(SCENARIO_NAME).unwrap())
            .unwrap()
    );
    let scenario = call(
        &mut editor,
        &mut reader,
        "world.singleton.get",
        json!({"name":SCENARIO_NAME}),
    )
    .unwrap();
    assert_eq!(
        scenario["value"],
        json!({"starting_gold":250,"reactor_hp":80})
    );
    let original = call(&mut editor, &mut reader, "scene.save", J::Null).unwrap();
    let history = call(&mut editor, &mut reader, "history.list", J::Null).unwrap();
    for (method, fields) in [
        ("editor.gameplay.control", json!({"action":"play"})),
        (
            "editor.gameplay.submit",
            json!({"intent":"act","payload":{"action":SUMMON,"source_cell":0,"target_cell":0}}),
        ),
    ] {
        let description = describe(&mut editor, &mut reader);
        let request = reader.mutation(&description, fields);
        assert_eq!(
            call(&mut editor, &mut reader, method, request)
                .unwrap_err()
                .code,
            PERMISSION_DENIED
        );
    }
    assert_eq!(
        call(&mut editor, &mut reader, "sim.state", J::Null).unwrap()["mode"],
        "edit"
    );
    // Edit intentionally has no gameplay frame stream. Verify the exact initial
    // bake through the host's typed admission path, independently of socket time.
    // The authenticated Scenario read above verifies the live host's source.
    let initial_doc = orr_remote::coop_defense::document(scene).unwrap();
    let initial = *initial_doc.frame().singleton::<DefenseRun>();
    assert_eq!(initial.gold, [250; 2]);
    assert_eq!(initial.reactor_hp, 80);
    control(&mut editor, &mut writer, json!({"action":"play"}));
    wait_read(&mut editor, &mut writer, "sim.state", J::Null, |v| {
        v["playing"] == true
    });
    wait_read(
        &mut editor,
        &mut writer,
        "editor.gameplay.describe",
        json!({}),
        |v| v["can_claim"] == true,
    );
    control(
        &mut editor,
        &mut writer,
        json!({"action":"claim","player":0}),
    );
    // Deliberately cross the AI board's first summon at tick 45. Aggregate
    // summons and net gold are not valid player-command oracles under WS delay.
    wait_read(&mut editor, &mut writer, "sim.state", J::Null, |v| {
        v["head_tick"].as_u64().is_some_and(|tick| tick >= 46)
    });
    let description = wait_read(
        &mut editor,
        &mut writer,
        "editor.gameplay.describe",
        json!({}),
        |v| v["can_submit"] == true,
    );
    // Run is deliberately not an authored/reflected singleton. Inspect the
    // existing public frame evidence instead of inventing a runtime read RPC.
    let before = *editor
        .gameplay_frame_evidence()
        .unwrap()
        .snapshot()
        .predicted()
        .singleton::<DefenseRun>();
    assert_eq!(
        editor
            .gameplay_frame_evidence()
            .unwrap()
            .snapshot()
            .predicted()
            .dense::<DefenseUnit>()
            .1
            .iter()
            .filter(|unit| unit.board == 0 && unit.active != 0)
            .count(),
        0
    );
    let request = writer.mutation(
        &description,
        json!({"intent":"act","payload":{"action":SUMMON,"source_cell":0,"target_cell":0}}),
    );
    let accepted = call(&mut editor, &mut writer, "editor.gameplay.submit", request).unwrap();
    let command = &accepted["command_id"];
    assert!(command.as_str().unwrap().parse::<u64>().unwrap() > 0);
    let completed = wait_read(
        &mut editor,
        &mut writer,
        "editor.gameplay.status",
        json!({"binding":description["binding"],"connection":description["connection"],"command_id":command}),
        |v| {
            assert!(matches!(
                v["status"].as_str(),
                Some("pending" | "consumed" | "completed")
            ));
            v["status"] == "completed"
        },
    );
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
        assert!(completed["consumed"].get(field).is_some());
        assert_eq!(completed["consumed"][field], completed["neutral"][field]);
    }
    let after = *editor
        .gameplay_frame_evidence()
        .unwrap()
        .snapshot()
        .predicted()
        .singleton::<DefenseRun>();
    assert!(after.last_sequence > before.last_sequence);
    // Exactly one player-board unit proves this socket command took effect,
    // independently of AI summons, passive income and kill rewards. Exact
    // spending is covered by deterministic core/editor tests, not this clock.
    assert_eq!(
        editor
            .gameplay_frame_evidence()
            .unwrap()
            .snapshot()
            .predicted()
            .dense::<DefenseUnit>()
            .1
            .iter()
            .filter(|unit| unit.board == 0 && unit.active != 0)
            .count(),
        1
    );
    control(&mut editor, &mut writer, json!({"action":"stop"}));
    wait_read(&mut editor, &mut writer, "sim.state", J::Null, |v| {
        v["mode"] == "edit"
    });
    assert_eq!(
        call(&mut editor, &mut reader, "scene.save", J::Null).unwrap(),
        original
    );
    assert_eq!(
        call(&mut editor, &mut reader, "history.list", J::Null).unwrap(),
        history
    );
    assert_eq!(
        call(
            &mut editor,
            &mut reader,
            "world.singleton.get",
            json!({"name":SCENARIO_NAME})
        )
        .unwrap()["value"],
        scenario["value"]
    );
    assert_eq!(std::fs::read_to_string(&scene_path).unwrap(), scene);
    assert_eq!(std::fs::read_to_string(&manifest_path).unwrap(), manifest);
    // Clients, listener and temporary project drop in that order. No environment
    // credentials, external services, production auth changes or mutation retries.
}
