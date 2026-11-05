//! CPU acceptance for the existing language-neutral observation endpoint.
//! The registered editor already receives typed frames. This separate ERP
//! consumer only reads protocol responses and queues, never typed game state.
//! Command status proves consumption/neutral completion; separately observed
//! state proves outcomes. OVS is not an epoch-fenced command receipt.
#![allow(
    clippy::disallowed_types,
    reason = "Wall-clock deadlines bound CPU ERP reproduction, not simulation"
)]

use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

use orr_editor::{registered_project::PreparedRegisteredProject, Editor, HostSpec};
use orr_remote::{Caps, ErpClient, RpcError, NOT_FOUND, PERMISSION_DENIED};
use serde_json::{json, Value as J};

// Local SDK bootstrap and the ordinary editor pump are the only native access.
// Every observation and lifecycle request below goes through the same host's ERP.
fn call(
    editor: &mut Editor,
    client: &mut ErpClient,
    method: &str,
    params: J,
) -> Result<J, RpcError> {
    let id = client.post(method, params).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.pump();
        client.poll().unwrap();
        if let Some(index) = client
            .responses
            .iter()
            .position(|(response_id, _)| *response_id == id)
        {
            return client.responses.remove(index).unwrap().1;
        }
        assert!(Instant::now() < deadline, "ERP response deadline: {method}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn control(
    editor: &mut Editor,
    client: &mut ErpClient,
    key: &str,
    action: &str,
) -> Result<J, RpcError> {
    let description = call(editor, client, "editor.gameplay.describe", json!({})).unwrap();
    call(
        editor,
        client,
        "editor.gameplay.control",
        json!({
            "binding":description["binding"], "connection":description["connection"],
            "scope":description["scope"], "request_key":key, "timeout_ms":5000,
            "action":action,
        }),
    )
}

fn assert_authoring_boundary(editor: &mut Editor, reader: &mut ErpClient) {
    let types = call(editor, reader, "registry.types", J::Null).unwrap();
    let types = types["types"].as_array().unwrap();
    let mut names: Vec<_> = types
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "CoopDefense::Campaign",
            "CoopDefense::EnemyHealth",
            "CoopDefense::Scenario",
            "CoopDefense::WaveComposition",
            "CoopDefense::WaveCounts",
            "CoopDefense::WavePacing",
        ],
        "the discoverable value registry only contains authored configuration"
    );
    assert!(types.iter().all(|entry| entry["kind"] == "singleton"));

    let scenario = call(
        editor,
        reader,
        "world.singleton.get",
        json!({"name":"CoopDefense::Scenario"}),
    )
    .unwrap();
    assert!(scenario["value"]["starting_gold"].as_u64().is_some());
    assert!(scenario["value"].get("gold").is_none());
    // Starting gold is authored configuration, not current spendable gold.
    for (method, params) in [
        ("registry.schema", json!({"type":"CoopDefense::Run"})),
        ("world.singleton.get", json!({"name":"CoopDefense::Run"})),
    ] {
        let error = call(editor, reader, method, params).unwrap_err();
        assert_eq!(error.code, NOT_FOUND);
        assert_eq!(error.kind(), Some("unknown_type"));
    }
}

fn subscribe_viewstream(
    editor: &mut Editor,
    reader: &mut ErpClient,
    source: &str,
    identity: &J,
) -> J {
    // Baseline failure must be the missing hook, not missing registration,
    // permission, or a made-up RPC. Subscribe on the SAME observing connection.
    let subscribed = call(
        editor,
        reader,
        "watch.subscribe",
        json!({"topics":["viewstream"], "source":source, "max_fps":1000}),
    )
    .unwrap_or_else(|error| {
        panic!("registered Stormglass {source} needs self-described runtime observation: {error}")
    });
    assert!(subscribed["topics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|topic| topic == "viewstream"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let schema = loop {
        editor.pump();
        reader.poll().unwrap();
        if let Some(index) = reader
            .notifications
            .iter()
            .position(|message| message["method"] == "watch.viewstream.schema")
        {
            break reader.notifications.remove(index).unwrap()["params"].clone();
        }
        assert!(Instant::now() < deadline, "viewstream schema deadline");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(schema["format"], "orrery.viewstream");
    assert_eq!(schema["version"], 1);
    assert_eq!(schema["game"], identity["game"]);
    assert_eq!(schema["build_id"], identity["build_id"]);
    assert_eq!(schema["endian"], "little");
    assert_eq!(schema["frame"]["magic"], "OVS1");
    assert!(!schema["kinds"].as_array().unwrap().is_empty());
    schema
}

fn assert_viewstream(editor: &mut Editor, reader: &mut ErpClient, source: &str, identity: &J) {
    let schema = subscribe_viewstream(editor, reader, source, identity);

    // An in-process foreign consumer receives the same OVS bytes as a socket
    // client, not a Rust LocalFrame. This checks transport/format only; it does
    // not equate a frame or a command receipt with a successful game outcome.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.pump();
        reader.poll().unwrap();
        assert!(
            reader.local_frames.is_empty(),
            "no typed-frame observation fallback"
        );
        while let Some(bytes) = reader.frames.pop_front() {
            assert_eq!(bytes.get(..4), Some(b"OVS1".as_slice()));
            if bytes.get(6) == Some(&1) {
                assert_eq!(bytes.get(4..6), Some([1, 0].as_slice()));
                assert!(bytes.len() >= schema["frame"]["header_len"].as_u64().unwrap() as usize);
                return;
            }
        }
        assert!(Instant::now() < deadline, "viewstream frame deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn registered_observation(source: &str) {
    let directory = tempfile::tempdir().unwrap();
    let manifest_path = directory.path().join("project.json");
    std::fs::write(
        &manifest_path,
        include_str!("../scenes/stormglass.project.json"),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("stormglass.scene.yaml"),
        include_str!("../scenes/stormglass.scene.yaml"),
    )
    .unwrap();
    let prepared = PreparedRegisteredProject::open(&manifest_path, super::registry()).unwrap();
    assert!(matches!(
        prepared.host_spec(),
        HostSpec::PreparedRegistered { listen: None, .. }
    ));
    let mut editor = Editor::open_registered_project(prepared).unwrap();
    editor.sync();
    let mut reader = editor
        .gameplay_api_client("observation-reader", Caps::parse("read").unwrap())
        .unwrap();
    let mut controller = editor
        .gameplay_api_client(
            "observation-controller",
            Caps::parse("read,sim_control").unwrap(),
        )
        .unwrap();

    let discovery = call(&mut editor, &mut reader, "rpc.discover", J::Null).unwrap();
    let description = call(
        &mut editor,
        &mut reader,
        "editor.gameplay.describe",
        json!({}),
    )
    .unwrap();
    assert_eq!(discovery["engine"]["game"], "CoopDefense");
    assert_eq!(description["game"], discovery["engine"]["game"]);
    assert_eq!(description["build_id"], discovery["engine"]["build_id"]);
    for method in [
        "registry.schema",
        "world.singleton.get",
        "watch.subscribe",
        "editor.gameplay.control",
    ] {
        assert!(discovery["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == method));
    }
    assert_eq!(description["mode"], "edit");
    assert_authoring_boundary(&mut editor, &mut reader);
    assert_eq!(
        control(&mut editor, &mut reader, "1", "play")
            .unwrap_err()
            .code,
        PERMISSION_DENIED
    );
    if source == "view" {
        assert_viewstream(&mut editor, &mut reader, source, &discovery["engine"]);
        return;
    }
    assert_eq!(
        control(&mut editor, &mut controller, "1", "play").unwrap()["status"],
        "queued"
    );

    // Wait for actual Play progress via ERP, rather than mistaking the queued
    // control ACK for a live session or inspecting the editor's typed frame.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = call(&mut editor, &mut reader, "sim.state", J::Null).unwrap();
        if state["mode"] == "play"
            && state["playing"] == true
            && state["head_tick"].as_u64().is_some_and(|tick| tick > 0)
        {
            break;
        }
        assert!(Instant::now() < deadline, "ERP Play progress deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    let live = call(
        &mut editor,
        &mut reader,
        "editor.gameplay.describe",
        json!({}),
    )
    .unwrap();
    assert_eq!(
        live["binding"], description["binding"],
        "same registered editor binding"
    );
    assert_eq!(
        live["connection"], description["connection"],
        "same observing connection"
    );
    assert_eq!(live["mode"], "play");
    assert_authoring_boundary(&mut editor, &mut reader);
    assert_viewstream(&mut editor, &mut reader, source, &discovery["engine"]);
}

#[test]
fn registered_editor_edit_view_advertises_language_neutral_observation() {
    registered_observation("view");
}

#[test]
fn registered_editor_play_sim_advertises_language_neutral_observation() {
    registered_observation("sim");
}

#[derive(Clone, Debug)]
struct Record {
    slot: u64,
    kind: String,
    values: BTreeMap<String, u32>,
}
impl Record {
    fn get(&self, name: &str) -> u32 {
        *self
            .values
            .get(name)
            .unwrap_or_else(|| panic!("missing {}.{name}", self.kind))
    }
}

#[derive(Clone, Debug)]
struct Observation {
    tick: u64,
    flags: u8,
    records: Vec<Record>,
}
impl Observation {
    fn run(&self) -> &Record {
        let mut found = self.records.iter().filter(|record| record.kind == "run");
        let run = found.next().expect("one run summary");
        assert!(found.next().is_none());
        run
    }
    fn cell(&self, board: u32, cell: u32) -> &Record {
        self.records
            .iter()
            .find(|record| {
                record.kind.starts_with("cell.")
                    && record.get("board") == board
                    && record.get("cell") == cell
            })
            .expect("advertised board cell")
    }
    fn units(&self, board: u32) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|record| {
                record.kind.starts_with("cell.")
                    && record.kind != "cell.empty"
                    && record.get("board") == board
            })
            .collect()
    }
    fn enemies(&self) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|record| record.kind.starts_with("enemy."))
            .collect()
    }
    fn checksum(&self) -> String {
        let sum = u64::from(self.run().get("checksum_lo"))
            | (u64::from(self.run().get("checksum_hi")) << 32);
        format!("0x{sum:016x}")
    }
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .expect("bounded wire word")
            .try_into()
            .unwrap(),
    )
}
fn long_word(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .expect("bounded wire u64")
            .try_into()
            .unwrap(),
    )
}

// Independent foreign-style reader for documented OVS1. No game/simulation
// structs, bytemuck casts, extractor access or typed local-frame payloads.
fn decode(schema: &J, bytes: &[u8]) -> Option<Observation> {
    assert!(bytes.len() >= 8 && bytes.len() <= 64 * 1024);
    assert_eq!(&bytes[..4], b"OVS1");
    assert_eq!(&bytes[4..6], &[1, 0]);
    match bytes[6] {
        2 => return None, // The advertised event payload is deliberately opaque.
        1 => {}
        other => panic!("unexpected OVS message type {other}"),
    }
    assert_eq!(schema["frame"]["header_len"], 56);
    assert_eq!(schema["frame"]["record_len"], 48);
    assert!(bytes.len() >= 56);
    let count = word(bytes, 48) as usize;
    assert!(
        (25..=89).contains(&count),
        "one summary, 24 cells, up to 64 enemies"
    );
    let mut properties = 56 + 48 * count;
    assert_eq!(properties + word(bytes, 52) as usize, bytes.len());
    let kinds = schema["kinds"].as_array().unwrap();
    let mut records = Vec::new();
    for index in 0..count {
        let start = 56 + 48 * index;
        let slot = long_word(bytes, start);
        assert!(!records.iter().any(|record: &Record| record.slot == slot));
        let kind_id = u16::from_le_bytes(bytes[start + 8..start + 10].try_into().unwrap());
        let kind = kinds
            .iter()
            .find(|kind| kind["id"] == kind_id)
            .expect("schema-defined kind");
        let mut values = BTreeMap::new();
        for property in kind["props"].as_array().unwrap() {
            assert_eq!(
                property["type"], "u32",
                "state properties remain exact integer words"
            );
            let name = property["name"].as_str().unwrap().to_owned();
            assert!(values.insert(name, word(bytes, properties)).is_none());
            properties += 4;
        }
        records.push(Record {
            slot,
            kind: kind["name"].as_str().unwrap().to_owned(),
            values,
        });
    }
    assert_eq!(properties, bytes.len());
    let observation = Observation {
        tick: long_word(bytes, 8),
        flags: bytes[7],
        records,
    };
    assert_eq!(
        long_word(bytes, 16),
        observation.tick,
        "local host is fully verified"
    );
    let run = observation.run();
    assert_eq!(
        run.slot,
        u64::from(u32::MAX),
        "reserved nonvisual summary ID"
    );
    assert_eq!(run.get("playing") + run.get("won") + run.get("lost"), 1);
    assert_eq!((run.get("board_cols"), run.get("board_rows")), (4, 3));
    for board in 0..2 {
        for cell in 0..12 {
            let value = observation.cell(board, cell);
            if value.kind == "cell.empty" {
                assert_eq!(
                    (
                        value.get("unit_id"),
                        value.get("tier"),
                        value.get("cooldown")
                    ),
                    (0, 0, 0)
                );
            } else {
                assert!(value.get("unit_id") > 0 && value.get("tier") > 0);
            }
        }
    }
    assert_eq!(
        observation
            .records
            .iter()
            .filter(|record| record.kind.starts_with("cell."))
            .count(),
        24
    );
    Some(observation)
}

struct Fixture {
    editor: Editor,
    reader: ErpClient,
    driver: ErpClient,
    key: u64,
    schema: J,
    identity: J,
    latest: Option<Observation>,
    opaque_batches: usize,
    original: J,
    history: J,
}
impl Fixture {
    fn new(scene: &str) -> (tempfile::TempDir, Self) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("project.json"),
            include_str!("../scenes/stormglass.project.json"),
        )
        .unwrap();
        std::fs::write(directory.path().join("stormglass.scene.yaml"), scene).unwrap();
        let fixture = Self::open(directory.path());
        (directory, fixture)
    }
    fn open(root: &Path) -> Self {
        let prepared =
            PreparedRegisteredProject::open(root.join("project.json"), super::registry()).unwrap();
        assert!(matches!(
            prepared.host_spec(),
            HostSpec::PreparedRegistered { listen: None, .. }
        ));
        let mut editor = Editor::open_registered_project(prepared).unwrap();
        editor.sync();
        let mut reader = editor
            .gameplay_api_client("state-reader", Caps::parse("read").unwrap())
            .unwrap();
        let driver = editor
            .gameplay_api_client("state-driver", Caps::parse("read,sim_control").unwrap())
            .unwrap();
        let identity =
            call(&mut editor, &mut reader, "rpc.discover", J::Null).unwrap()["engine"].clone();
        let original = call(&mut editor, &mut reader, "scene.save", J::Null).unwrap();
        let history = call(&mut editor, &mut reader, "history.list", J::Null).unwrap();
        let schema = subscribe_viewstream(&mut editor, &mut reader, "view", &identity);
        assert_eq!(
            schema["events"],
            json!([{"id":0,"name":"defense_event_opaque","payload_size":48}])
        );
        Self {
            editor,
            reader,
            driver,
            key: 0,
            schema,
            identity,
            latest: None,
            opaque_batches: 0,
            original,
            history,
        }
    }
    fn read(&mut self, method: &str, params: J) -> J {
        call(&mut self.editor, &mut self.reader, method, params).unwrap()
    }
    fn describe(&mut self) -> J {
        call(
            &mut self.editor,
            &mut self.driver,
            "editor.gameplay.describe",
            json!({}),
        )
        .unwrap()
    }
    fn request(&mut self, description: &J, fields: J) -> J {
        self.key += 1;
        let mut request = json!({"binding":description["binding"],"connection":description["connection"],
            "scope":description["scope"],"request_key":self.key.to_string(),"timeout_ms":5000});
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        request
    }
    fn control(&mut self, fields: J) {
        let description = self.describe();
        let request = self.request(&description, fields);
        let response = call(
            &mut self.editor,
            &mut self.driver,
            "editor.gameplay.control",
            request,
        )
        .unwrap();
        assert_eq!(response["status"], "queued");
    }
    fn observe(&mut self) {
        self.editor.pump();
        self.reader.poll().unwrap();
        assert!(
            self.reader.local_frames.is_empty(),
            "foreign observer never consumes typed frames"
        );
        while let Some(bytes) = self.reader.frames.pop_front() {
            if let Some(observation) = decode(&self.schema, &bytes) {
                self.latest = Some(observation);
            } else {
                self.opaque_batches += 1;
            }
        }
    }
    // Paused comparison only, within this established session. The two state
    // reads are not an atomic bundle and OVS carries no gameplay receipt epochs.
    fn settled(&mut self, mode: &str) -> Observation {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let before = self.read("sim.state", J::Null);
            self.observe();
            let after = self.read("sim.state", J::Null);
            let description = self.describe();
            if before["mode"] == mode
                && after["mode"] == mode
                && after["playing"] == false
                && before["playing"] == false
                && before["epoch"] == after["epoch"]
                && before["head_tick"] == after["head_tick"]
                && before["checksum"] == after["checksum"]
                && description["mode"] == mode
                && description["playing"] == false
                && description["phase"] == "off"
                && description["cleanup_pending"] == false
                && description["error"] == false
            {
                if let Some(observation) = &self.latest {
                    if after["head_tick"] == observation.tick
                        && after["checksum"] == observation.checksum()
                        && observation.flags & 4 != 0
                    {
                        return observation.clone();
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "settled {mode} observation: state={after}, describe={description}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn play_and_claim(&mut self) {
        self.control(json!({"action":"play"}));
        self.latest = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        let ready = loop {
            let before = self.describe();
            let state = self.read("sim.state", J::Null);
            self.observe();
            let after = self.describe();
            let tick = state["head_tick"].as_u64().unwrap();
            let expected_tick = tick.to_string();
            if before["scope"] == after["scope"]
                && before["binding"] == after["binding"]
                && after["can_claim"] == true
                && after["phase"] == "off"
                && after["cleanup_pending"] == false
                && after["error"] == false
                && after["mode"] == "play"
                && after["playing"] == true
                && state["mode"] == "play"
                && state["playing"] == true
                && state["session_mode"] == "record"
                && state["head_tick"] == state["last_tick"]
                && tick > 0
                && after["tick"].as_str() == Some(expected_tick.as_str())
                && self.latest.as_ref().is_some_and(|observation| {
                    observation.tick == tick
                        && observation.flags & 4 == 0
                        && state["checksum"] == observation.checksum()
                })
            {
                break after;
            }
            assert!(
                Instant::now() < deadline,
                "public claim readiness: state={state}, describe={after}"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        // Exactly one claim after agreement of public readiness and frontier.
        // A timing failure is a test failure, never repaired with claim retries.
        let claim = self.request(&ready, json!({"action":"claim","player":0}));
        assert_eq!(
            call(
                &mut self.editor,
                &mut self.driver,
                "editor.gameplay.control",
                claim
            )
            .unwrap()["status"],
            "queued"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let description = self.describe();
            if description["can_submit"] == true && description["phase"] == "active" {
                break;
            }
            assert_eq!(description["error"], false);
            assert!(Instant::now() < deadline, "submit readiness: {description}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn action(&mut self, action: u32, source: u32, target: u32) -> (J, Observation) {
        self.play_and_claim();
        let description = self.describe();
        let request = self.request(&description, json!({"intent":"act","payload":{"action":action,"source_cell":source,"target_cell":target}}));
        let accepted = call(
            &mut self.editor,
            &mut self.driver,
            "editor.gameplay.submit",
            request,
        )
        .unwrap();
        let id = accepted["command_id"].clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        let completed = loop {
            let status = call(&mut self.editor, &mut self.driver, "editor.gameplay.status",
                json!({"binding":accepted["binding"],"connection":accepted["connection"],"command_id":id})).unwrap();
            if status["status"] == "completed" {
                break status;
            }
            assert!(
                status["status"] == "pending" || status["status"] == "consumed",
                "unexpected command status {status}"
            );
            assert!(
                Instant::now() < deadline,
                "command completion deadline: {status}"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        for field in ["stream_generation", "play_epoch", "timeline_epoch"] {
            assert_eq!(completed["consumed"][field], completed["neutral"][field]);
        }
        let tick = |value: &J| value.as_str().unwrap().parse::<u64>().unwrap();
        assert!(tick(&completed["neutral"]["tick"]) > tick(&completed["consumed"]["tick"]));
        self.control(json!({"action":"pause"}));
        let observed = self.settled("play");
        assert!(observed.tick >= tick(&completed["neutral"]["tick"]));
        assert_eq!(u64::from(observed.run().get("last_sequence")), tick(&id));
        (completed, observed)
    }
    fn step(&mut self, n: u32) -> Observation {
        assert!((1..=60).contains(&n));
        let before = self.settled("play");
        self.control(json!({"action":"step","n":n}));
        let after = self.settled("play");
        assert_eq!(after.tick, before.tick + u64::from(n));
        after
    }
    fn stop(&mut self) -> Observation {
        self.control(json!({"action":"stop"}));
        let reset = self.settled("edit");
        let original = self.original.clone();
        let history = self.history.clone();
        assert_eq!(self.read("scene.save", J::Null), original);
        assert_eq!(self.read("history.list", J::Null), history);
        reset
    }
}

#[test]
fn registered_commands_have_wire_observable_outcomes_and_preserve_authored_source() {
    let scene = "schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario: {starting_gold: 10000, reactor_hp: 1000}\n";
    let (directory, mut fixture) = Fixture::new(scene);
    let initial = fixture.settled("edit");
    assert_eq!(initial.run().get("gold_board_0"), 10000);
    assert_eq!(initial.run().get("gold_board_1"), 10000);
    assert_eq!(initial.run().get("last_sequence"), 0);
    assert!(initial.units(0).is_empty());
    let (_, summoned) = fixture.action(1, 0, 0);
    assert_eq!(summoned.units(0).len(), 1);
    let unit_id = summoned.cell(0, 0).get("unit_id");
    let role = summoned.cell(0, 0).kind.clone();
    let original_slot = summoned.cell(0, 0).slot;
    let destination_slot = summoned.cell(0, 11).slot;

    let (_, moved) = fixture.action(2, 0, 11);
    assert_eq!(moved.cell(0, 0).kind, "cell.empty");
    assert_eq!(moved.cell(0, 0).slot, original_slot);
    assert_eq!(moved.cell(0, 11).slot, destination_slot);
    assert_eq!(moved.cell(0, 11).get("unit_id"), unit_id);
    assert_eq!(moved.cell(0, 11).kind, role);

    // Empty-source movement is consumed and neutralized but changes no unit.
    // This is a game-level no-op, not protocol rejection or command failure.
    let (no_op_status, no_op) = fixture.action(2, 0, 1);
    assert_eq!(no_op_status["status"], "completed");
    assert_eq!(no_op.units(0).len(), 1);
    assert_eq!(no_op.cell(0, 11).get("unit_id"), unit_id);
    assert_eq!(no_op.cell(0, 11).get("tier"), moved.cell(0, 11).get("tier"));
    assert_eq!(no_op.cell(0, 0).kind, "cell.empty");
    assert_eq!(no_op.cell(0, 1).kind, "cell.empty");

    let (_, upgraded) = fixture.action(3, 0, 0);
    assert_eq!(
        upgraded.run().get("upgrades_board_0"),
        no_op.run().get("upgrades_board_0") + 1
    );
    let (_, invested) = fixture.action(4, 0, 0);
    assert_eq!(
        invested.run().get("investment_board_0"),
        upgraded.run().get("investment_board_0") + 1
    );
    // Gold is observed exactly as a u32. Do not assert asynchronous cost deltas:
    // passive income may occur between the command and the paused observation.
    assert!(invested.run().get("gold_board_0") <= 99999);

    let mut board = invested;
    for _ in 0..6 {
        board = fixture.action(1, 0, 0).1;
    }
    assert_eq!(board.units(0).len(), 7);
    // Seven tier-one units among six advertised roles guarantee a pair. Choose
    // it from the observed wire vocabulary, never RNG or typed game inspection.
    let units = board.units(0);
    let (source, target) = units
        .iter()
        .enumerate()
        .find_map(|(index, source)| {
            units
                .iter()
                .skip(index + 1)
                .find(|target| {
                    source.kind == target.kind && source.get("tier") == target.get("tier")
                })
                .map(|target| (*source, *target))
        })
        .expect("seven units contain a same-role pair");
    let source_cell = source.get("cell");
    let target_cell = target.get("cell");
    let target_id = target.get("unit_id");
    let target_slot = target.slot;
    let tier = target.get("tier");
    let (_, merged) = fixture.action(2, source_cell, target_cell);
    assert_eq!(merged.units(0).len(), 6);
    assert_eq!(merged.cell(0, source_cell).kind, "cell.empty");
    assert_eq!(merged.cell(0, target_cell).slot, target_slot);
    assert_eq!(merged.cell(0, target_cell).get("unit_id"), target_id);
    assert_eq!(merged.cell(0, target_cell).get("tier"), tier + 1);

    let (_, restarted) = fixture.action(5, 0, 0);
    assert_eq!(
        restarted.run().get("generation"),
        merged.run().get("generation") + 1
    );
    assert_eq!(restarted.run().get("upgrades_board_0"), 0);
    assert_eq!(restarted.run().get("investment_board_0"), 0);
    assert_eq!(restarted.run().get("playing"), 1);
    assert!(restarted.units(0).is_empty());
    assert!(restarted.run().get("gold_board_0") >= 10000);
    assert!(u64::from(restarted.run().get("run_tick")) < restarted.tick);
    let stepped = fixture.step(1);
    assert_eq!(
        stepped.run().get("run_tick"),
        restarted.run().get("run_tick") + 1
    );
    assert_eq!(
        stepped.run().get("playing"),
        1,
        "run phase is not host playback state"
    );

    // Reconnect a read-only observer to this same host. The old connection fence
    // cannot be reused, and observing a new schema does not grant input authority.
    let old_description = fixture.read("editor.gameplay.describe", json!({}));
    let replacement = fixture
        .editor
        .gameplay_api_client("state-reader", Caps::parse("read").unwrap())
        .unwrap();
    drop(std::mem::replace(&mut fixture.reader, replacement));
    let stale = call(
        &mut fixture.editor,
        &mut fixture.reader,
        "editor.gameplay.describe",
        json!({"binding":old_description["binding"],"connection":old_description["connection"]}),
    )
    .unwrap_err();
    assert_eq!(stale.kind(), Some("editor_gameplay_stale"));
    let fresh = fixture.read("editor.gameplay.describe", json!({}));
    assert_eq!(fresh["binding"], old_description["binding"]);
    assert_ne!(fresh["connection"], old_description["connection"]);
    fixture.schema = subscribe_viewstream(
        &mut fixture.editor,
        &mut fixture.reader,
        "view",
        &fixture.identity,
    );
    fixture.latest = None;
    let reconnected = fixture.settled("play");
    assert_eq!(reconnected.tick, stepped.tick);
    assert_eq!(reconnected.checksum(), stepped.checksum());
    for (method, fields) in [
        (
            "editor.gameplay.control",
            json!({"action":"claim","player":0}),
        ),
        (
            "editor.gameplay.submit",
            json!({"intent":"act","payload":{"action":1,"source_cell":0,"target_cell":0}}),
        ),
    ] {
        let mut request = json!({"binding":fresh["binding"],"connection":fresh["connection"],
            "scope":fresh["scope"],"request_key":"1","timeout_ms":5000});
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert_eq!(
            call(&mut fixture.editor, &mut fixture.reader, method, request)
                .unwrap_err()
                .code,
            PERMISSION_DENIED
        );
    }
    let original = fixture.original.clone();
    let history = fixture.history.clone();
    let stopped = fixture.stop();
    assert_eq!(stopped.tick, 0);
    assert_eq!(stopped.run().get("gold_board_0"), 10000);
    assert_eq!(
        stopped.run().get("generation"),
        initial.run().get("generation")
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("stormglass.scene.yaml")).unwrap(),
        scene
    );
    assert_eq!(
        std::fs::read(directory.path().join("project.json")).unwrap(),
        include_bytes!("../scenes/stormglass.project.json").as_slice()
    );
    drop(fixture);
    let mut reopened = Fixture::open(directory.path());
    assert_eq!(reopened.original, original);
    assert_eq!(reopened.history, history);
    let fresh = reopened.settled("edit");
    assert_eq!(fresh.checksum(), initial.checksum());
    assert!(fresh.units(0).is_empty());
}

#[test]
fn registered_one_wave_win_loss_and_restart_are_observed_from_wire() {
    for (reactor_hp, expected_phase) in [(30, "won"), (1, "lost")] {
        // No purchased units. The one natural fast-enemy pair eventually leaks:
        // healthy reactors survive and finish the campaign; one HP loses.
        let scene = format!("schema: orr.scene/1\nentities: {{}}\nsingletons:\n  CoopDefense::Scenario: {{starting_gold: 0, reactor_hp: {reactor_hp}}}\n  CoopDefense::Campaign: {{last_wave: 1}}\n  CoopDefense::WaveCounts: {{pair_counts: [1,1,1,1,1,1,1,1,1,1]}}\n");
        let (_directory, mut fixture) = Fixture::new(&scene);
        fixture.settled("edit");
        // Semantic Step starts a paused session. No real-time wait, injected
        // state, typed test oracle or alternate simulation chooses the outcome.
        fixture.control(json!({"action":"step","n":1}));
        let mut observation = fixture.settled("play");
        let mut saw_enemies = false;
        while observation.run().get("playing") == 1 {
            assert!(observation.tick < 900, "natural one-wave outcome deadline");
            observation = fixture.step(60);
            if !observation.enemies().is_empty() {
                saw_enemies = true;
                assert_eq!(observation.enemies().len(), 2);
                for enemy in observation.enemies() {
                    assert_eq!(enemy.kind, "enemy.fast");
                    assert!(enemy.get("enemy_id") > 0);
                    assert!(enemy.get("lane") < 2);
                    assert_eq!((enemy.get("hp"), enemy.get("max_hp")), (39, 39));
                    assert!(enemy.get("progress") > 0 && enemy.get("progress") < 2000);
                }
            }
        }
        assert!(
            saw_enemies,
            "the stream exposed actual active enemies before the outcome"
        );
        assert!(
            fixture.opaque_batches > 0,
            "opaque event batches were skipped, not decoded as frames"
        );
        assert_eq!(observation.run().get(expected_phase), 1);
        assert_eq!(observation.run().get("last_wave"), 1);
        assert_eq!(
            observation.run().get("reactor_hp"),
            if reactor_hp == 30 { 26 } else { 0 }
        );
        assert!(observation.enemies().is_empty());
        let terminal_run_tick = observation.run().get("run_tick");
        let advanced = fixture.step(60);
        assert_eq!(
            advanced.run().get("run_tick"),
            terminal_run_tick,
            "terminal run tick freezes while engine tick advances"
        );
        assert_eq!(advanced.run().get(expected_phase), 1);
        let (_, restarted) = fixture.action(5, 0, 0);
        assert_eq!(
            restarted.run().get("generation"),
            advanced.run().get("generation") + 1
        );
        assert_eq!(restarted.run().get("playing"), 1);
        assert_eq!(
            (restarted.run().get("won"), restarted.run().get("lost")),
            (0, 0)
        );
        assert_eq!(restarted.run().get("max_reactor_hp"), reactor_hp);
        assert_eq!(restarted.run().get("last_wave"), 1);
        assert!(u64::from(restarted.run().get("run_tick")) < restarted.tick);
        fixture.stop();
    }
}
