//! Yard3D's view stream is served by a real ERP host over loopback sockets.
//!
//! These checks cover the public schema, raw per-player input, the authoritative
//! 3D snapshot, timeline discontinuities, replay, and the two ERP transports.
//! Local ERP play is a single authoritative session; it does not exercise the
//! multiplayer client's late-input rollback path.

// Socket deadlines and decoded view floats belong to this integration test,
// not to the deterministic simulation or its serialized state.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use orr_bridge::FrameView;
use orr_reflect::Value as ReflectValue;
use orr_remote::yard3d::{
    YARD_BUILD_ID, YARD_PLAYERS, YARD_SEED, YARD_TICK_RATE, spawn_yard3d_host, yard3d_doc,
    yard3d_types,
};
use orr_remote::{Auth, ErpClient, ServerConfig, codec};
use orr_sample::yard3d_game::{NoCommand, SPAWN_BALL, SPAWN_BOX, Yard3D, YardConfig, YardInput};
use orr_sample::yard3d_stream::{KIND_DYNAMIC, KIND_STATIC};
use orr_sample::yard3d_view::YardExtractor;
use orr_session::{ControlOp, PlaySession, ReplayReader};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use orr_view::{Extracted3, Extractor3, InterpMode, Shape3, fp_to_vec3};
use orr_viewstream::{
    FLAG_DISCONTINUITY, FLAG_PAUSED, HEADER_LEN, MODE_NONE, MODE_PREDICTION, MODE_SNAPSHOT,
    MSG_FRAME3D, Pose3, RECORD3D_LEN, SHAPE3_BOX, SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE,
    STYLE_CHECKER, VERSION_3D, ViewFrame3, color_to_u8, entity_id,
};
use serde_json::{Value as J, json};

const WAIT: Duration = Duration::from_secs(20);
const BODY: &str = "orr_physics3d::Body";

fn yard_config() -> YardConfig {
    YardConfig {
        rain_per_second: 0,
        max_entities: 256,
        ..YardConfig::new(4)
    }
}

fn host() -> orr_remote::LocalHost {
    spawn_yard3d_host(yard_config(), ServerConfig::new(Auth::DevNoAuth))
        .expect("start Yard3D ERP host")
}

fn subscribed(url: &str) -> ErpClient {
    let mut client = ErpClient::connect(url, None).expect("connect WebSocket ERP client");
    let result = client
        .call(
            "watch.subscribe",
            json!({"topics": ["viewstream"], "max_fps": 1000}),
        )
        .unwrap();
    assert_eq!(result["topics"], json!(["viewstream"]));
    client
}

fn wait_schema(client: &mut ErpClient) -> J {
    client
        .wait_notification("watch.viewstream.schema", WAIT)
        .expect("wait for schema notification")
        .expect("schema is sent on subscribe")["params"]
        .clone()
}

fn frame_at_after(
    client: &mut ErpClient,
    tick: u64,
    seq_after: u64,
    required_flags: u8,
    forbidden_flags: u8,
) -> (Vec<u8>, ViewFrame3) {
    let deadline = Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "no Yard3D frame for tick {tick} after seq {seq_after}"
        );
        let bytes = client
            .wait_frame(remaining)
            .expect("read WebSocket frame")
            .unwrap_or_else(|| panic!("no Yard3D frame for tick {tick} after seq {seq_after}"));
        let frame = ViewFrame3::decode(&bytes).expect("decode ViewFrame3");
        if frame.seq <= seq_after || frame.tick != tick {
            continue;
        }
        if frame.flags & required_flags != required_flags || frame.flags & forbidden_flags != 0 {
            continue;
        }
        assert_eq!(frame.flags & required_flags, required_flags);
        assert_eq!(frame.flags & forbidden_flags, 0);
        return (bytes, frame);
    }
}

fn input(buttons: u32) -> YardInput {
    YardInput {
        buttons,
        _pad: 0,
        origin: [0, 1_000, 2_000],
        dir: [0, -447, -894],
    }
}

fn input_hex(input: &YardInput) -> String {
    codec::hex_encode(bytemuck::bytes_of(input))
}

fn component_scalar(client: &mut ErpClient, entity: &str, path: &str) -> f64 {
    client
        .call(
            "world.get",
            json!({"entity": entity, "component": BODY, "path": path}),
        )
        .unwrap_or_else(|e| panic!("world.get {path}: {e}"))["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("world.get {path} did not return a number"))
}

fn entity_handle(id: u64) -> String {
    format!("{}v{}", id as u32, (id >> 32) as u32)
}

fn assert_frame_layout(bytes: &[u8], frame: &ViewFrame3) {
    assert_eq!(&bytes[..4], b"OVS1");
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), VERSION_3D);
    assert_eq!(bytes[6], MSG_FRAME3D);
    assert_eq!(
        bytes.len(),
        HEADER_LEN + frame.entities.len() * RECORD3D_LEN + frame.props.len()
    );
}

fn assert_unit_quaternion(q: [f32; 4]) {
    let norm2 = q.iter().map(|v| v * v).sum::<f32>();
    assert!(
        (norm2 - 1.0).abs() < 0.002,
        "not a unit quaternion: {q:?} (norm²={norm2})"
    );
}

fn assert_matches_yard_extractor(
    client: &mut ErpClient,
    wire: &ViewFrame3,
    previous: &orr_ecs::Frame,
    current: &orr_ecs::Frame,
) {
    let types = yard3d_types();
    let mut before = Vec::<Extracted3>::new();
    let mut now = Vec::<Extracted3>::new();
    YardExtractor.extract(FrameView::of(previous), &mut before);
    YardExtractor.extract(FrameView::of(current), &mut now);
    assert_eq!(wire.entities.len(), now.len());

    let mut expected_props = Vec::new();
    for (actual, item) in wire.entities.iter().zip(&now) {
        let prior = before
            .iter()
            .find(|candidate| candidate.entity == item.entity)
            .unwrap_or(item);
        assert_eq!(actual.id, entity_id(item.entity));
        assert_eq!(
            actual.kind,
            match item.mode {
                InterpMode::Prediction => KIND_DYNAMIC,
                _ => KIND_STATIC,
            }
        );
        let (shape, size) = match item.style.shape {
            Shape3::Sphere { radius } => (SHAPE3_SPHERE, [radius, 0.0, 0.0]),
            Shape3::Box { half } => (SHAPE3_BOX, half),
            Shape3::Capsule {
                half_length,
                radius,
            } => (SHAPE3_CAPSULE, [radius, half_length, 0.0]),
            Shape3::Plane { half_x, half_z } => (SHAPE3_PLANE, [half_x, 0.0, half_z]),
        };
        assert_eq!(actual.shape, shape);
        assert_eq!(actual.size, size);
        assert_eq!(
            actual.mode,
            match item.mode {
                InterpMode::Prediction => MODE_PREDICTION,
                InterpMode::Snapshot => MODE_SNAPSHOT,
                InterpMode::None => MODE_NONE,
            }
        );
        assert_eq!(
            actual.rgba,
            item.style.color.map(color_to_u8),
            "entity {} color",
            actual.id
        );
        assert_eq!(actual.roughness, color_to_u8(item.style.roughness));
        assert_eq!(actual.metallic, color_to_u8(item.style.metallic));
        assert_eq!(
            actual.style_flags,
            if item.style.checker { STYLE_CHECKER } else { 0 }
        );
        assert_eq!(
            actual.prev,
            Pose3 {
                pos: prior.transform.pos.to_array(),
                rot: prior.transform.rot.to_array(),
            },
            "entity {} previous pose",
            actual.id
        );
        assert_eq!(
            actual.cur,
            Pose3 {
                pos: item.transform.pos.to_array(),
                rot: item.transform.rot.to_array(),
            },
            "entity {} current pose",
            actual.id
        );
        if actual.kind == KIND_DYNAMIC {
            let handle = entity_handle(actual.id);
            let exact_velocity = match types
                .get_field(current, item.entity, BODY, "vel")
                .expect("headless Body velocity is registered")
            {
                ReflectValue::Vec3(value) => fp_to_vec3(value),
                other => panic!("Body.vel is not a fixed-point vector: {other:?}"),
            };
            let actual_velocity = ["x", "y", "z"]
                .map(|axis| component_scalar(client, &handle, &format!("vel.{axis}")));
            for (actual, expected) in actual_velocity.into_iter().zip(exact_velocity.to_array()) {
                let tolerance =
                    1.0 / orr_fp::FP::SCALE as f64 + f64::from(expected.abs() * f32::EPSILON);
                assert!(
                    (actual - f64::from(expected)).abs() <= tolerance,
                    "ERP velocity component {actual} differs from headless fixed-point value {expected}"
                );
            }
            let speed = exact_velocity.length();
            expected_props.extend_from_slice(&speed.to_bits().to_le_bytes());
        }
    }
    assert_eq!(
        wire.props, expected_props,
        "all per-entity speed properties"
    );
}

#[test]
fn yard3d_schema_snapshot_inputs_and_replay_are_authoritative() {
    let config = yard_config();
    let doc = yard3d_doc(config).expect("build the same deterministic Yard3D document");
    assert_eq!(doc.seed(), YARD_SEED);
    let host = host();
    let mut client = subscribed(host.url().expect("listening ERP URL"));

    let schema = wait_schema(&mut client);
    assert_eq!(schema["format"], "orrery.viewstream");
    assert_eq!(schema["game"], "Yard3D");
    assert_eq!(schema["version"], 2);
    assert_eq!(schema["build_id"], format!("0x{YARD_BUILD_ID:016x}"));
    assert_eq!(schema["tick_rate"], YARD_TICK_RATE);
    assert_eq!(schema["player_count"], YARD_PLAYERS);
    assert_eq!(schema["frame3d"]["message_type"], MSG_FRAME3D);
    assert_eq!(schema["frame3d"]["record_len"], RECORD3D_LEN);
    assert_eq!(schema["frame3d"]["header_len"], HEADER_LEN);
    assert_eq!(schema["frame3d"]["style_flags"]["checker"], STYLE_CHECKER);
    assert_eq!(
        schema["input"]["size"], 32,
        "YardInput is the per-player wire sample"
    );
    assert_eq!(
        schema["command"]["size"], 4,
        "NoCommand still has its explicit four-byte encoding"
    );

    let started = client.call("sim.start", json!({})).unwrap();
    assert_eq!(started["mode"], "play");
    assert_eq!(started["playing"], false, "new sessions start paused");
    assert_eq!(started["head_tick"], 0);
    let (initial_bytes, initial) =
        frame_at_after(&mut client, 0, 0, FLAG_DISCONTINUITY | FLAG_PAUSED, 0);
    assert_frame_layout(&initial_bytes, &initial);
    assert_eq!(initial.tick, 0);
    assert!(initial.has(FLAG_DISCONTINUITY) && initial.has(FLAG_PAUSED));
    assert!(
        initial.entities.len() > 40,
        "the ERP document includes the Yard3D fixtures"
    );
    assert!(
        initial
            .entities
            .iter()
            .any(|e| e.shape == SHAPE3_PLANE && e.style_flags & STYLE_CHECKER != 0)
    );
    assert!(initial.entities.iter().any(|e| e.shape == SHAPE3_BOX));
    assert!(initial.entities.iter().any(|e| e.shape == SHAPE3_SPHERE));
    assert!(initial.entities.iter().all(|e| e.prev == e.cur));
    for entity in &initial.entities {
        assert_unit_quaternion(entity.cur.rot);
    }

    let malformed = client.call_err("sim.input", json!({"player": 0, "input": "00"}));
    assert_eq!(malformed.code, orr_remote::INVALID_PARAMS);
    assert!(
        malformed.message.contains("32 bytes"),
        "{}",
        malformed.message
    );
    let malformed_hex = client.call_err("sim.input", json!({"player": 0, "input": "not-hex"}));
    assert_eq!(malformed_hex.code, orr_remote::INVALID_PARAMS);
    let missing_player = client.call_err(
        "sim.input",
        json!({"player": YARD_PLAYERS, "input": input_hex(&input(0))}),
    );
    assert_eq!(missing_player.code, orr_remote::INVALID_PARAMS);
    let structured_unavailable =
        client.call_err("sim.input_value", json!({"player": 0, "value": {}}));
    assert_eq!(structured_unavailable.code, orr_remote::INVALID_STATE);
    assert_eq!(structured_unavailable.kind(), Some("input_unavailable"));
    let malformed_command = client.call_err("sim.command", json!({"player": 0, "command": "0000"}));
    assert_eq!(malformed_command.code, orr_remote::INVALID_PARAMS);
    assert_eq!(
        client
            .call("sim.command", json!({"player": 0, "command": "00000000"}))
            .unwrap()["ok"],
        true
    );

    let box_input = input(SPAWN_BOX);
    let ball_input = input(SPAWN_BALL);
    for (player, sample) in [(0, &box_input), (1, &ball_input)] {
        client
            .call(
                "sim.input",
                json!({"player": player, "input": input_hex(sample)}),
            )
            .unwrap();
    }
    let stepped = client.call("sim.step", json!({"n": 8})).unwrap();
    assert_eq!(stepped["head_tick"], 8);
    let (bytes, frame) = frame_at_after(&mut client, 8, initial.seq, 0, FLAG_DISCONTINUITY);
    assert_eq!(frame.tick, 8);
    assert_frame_layout(&bytes, &frame);
    assert!(!frame.has(FLAG_DISCONTINUITY));
    assert!(
        frame.entities.iter().any(|e| e.prev != e.cur),
        "moving bodies expose adjacent-tick poses"
    );

    let initial_ids: BTreeSet<_> = initial.entities.iter().map(|e| e.id).collect();
    let current_ids: BTreeSet<_> = frame.entities.iter().map(|e| e.id).collect();
    assert_eq!(
        current_ids.len(),
        frame.entities.len(),
        "entity index/version ids are unique in one snapshot"
    );
    assert!(
        initial_ids.is_subset(&current_ids),
        "stable entity index/version identities persist"
    );
    assert!(
        current_ids.iter().any(|id| !initial_ids.contains(id)),
        "player input spawns versioned entities"
    );
    for entity in &frame.entities {
        assert_unit_quaternion(entity.cur.rot);
    }

    let state = client.call("sim.state", J::Null).unwrap();
    let checksum = client.call("sim.checksum", J::Null).unwrap();
    assert_eq!(state["head_tick"], frame.tick);
    assert_eq!(state["checksum"], checksum["checksum"]);
    let dynamic = frame
        .entities
        .iter()
        .find(|e| e.mode == 0)
        .expect("a predicted dynamic body");
    let handle = entity_handle(dynamic.id);
    for (axis, index) in [("x", 0), ("y", 1), ("z", 2)] {
        let got = component_scalar(&mut client, &handle, &format!("pos.{axis}"));
        assert!(
            (got - f64::from(dynamic.cur.pos[index])).abs() < 0.002,
            "ERP body position differs on {axis}: {got} vs {}",
            dynamic.cur.pos[index]
        );
    }
    for (axis, index) in [("x", 0), ("y", 1), ("z", 2), ("w", 3)] {
        let got = component_scalar(&mut client, &handle, &format!("rot.{axis}"));
        assert!(
            (got - f64::from(dynamic.cur.rot[index])).abs() < 0.002,
            "ERP body quaternion differs on {axis}: {got} vs {}",
            dynamic.cur.rot[index]
        );
    }

    // The public raw input requests must produce the same deterministic state
    // as stepping the exact ERP document frame in a headless simulation.
    let mut reference =
        Simulation::<Yard3D>::from_frame(doc.frame(), YARD_TICK_RATE, YARD_BUILD_ID).unwrap();
    let mut previous_frame = None;
    for tick in 1..=8 {
        let mut inputs = TickInputs::<YardInput, NoCommand>::new(tick, YARD_PLAYERS);
        inputs.set_input(PlayerSlot(0), box_input);
        inputs.set_input(PlayerSlot(1), ball_input);
        reference.step(&inputs);
        if tick == 7 {
            previous_frame = Some(reference.frame().clone());
        }
    }
    assert_matches_yard_extractor(
        &mut client,
        &frame,
        previous_frame.as_ref().expect("headless tick 7 frame"),
        reference.frame(),
    );
    assert_eq!(
        reference.checksum(),
        orr_remote::wire::parse_checksum(&state["checksum"]).unwrap()
    );

    let stopped = client
        .call("sim.stop", json!({"include_replay": true}))
        .unwrap();
    assert_eq!(stopped["tick"], 8);
    let replay_bytes = codec::b64_decode(
        stopped["replay"]
            .as_str()
            .expect("replay payload is base64"),
    )
    .unwrap();
    let replay_header = ReplayReader::<Yard3D>::parse(&replay_bytes).unwrap().header;
    assert_eq!(replay_header.build_hash, reference.build_hash());
    let mut replay = PlaySession::<Yard3D>::open_replay(&replay_bytes, config, YARD_BUILD_ID)
        .expect("open the ERP recording headlessly");
    replay.control(ControlOp::Seek(8));
    assert_eq!(replay.head_tick(), 8);
    assert_eq!(replay.frame().checksum(), reference.checksum());
    assert_eq!(stopped["checksum"], state["checksum"]);

    let restarted = client.call("sim.start", json!({})).unwrap();
    assert_eq!(restarted["head_tick"], 0);
    let (_, fresh) = frame_at_after(
        &mut client,
        0,
        frame.seq,
        FLAG_DISCONTINUITY | FLAG_PAUSED,
        0,
    );
    assert!(fresh.has(FLAG_DISCONTINUITY) && fresh.has(FLAG_PAUSED));
    assert!(fresh.entities.iter().all(|e| e.prev == e.cur));
}

#[test]
fn pause_step_seek_and_stop_publish_timeline_jumps() {
    let host = host();
    let mut client = subscribed(host.url().unwrap());
    let _schema = wait_schema(&mut client);
    client.call("sim.start", json!({})).unwrap();
    let (_, zero) = frame_at_after(&mut client, 0, 0, FLAG_DISCONTINUITY | FLAG_PAUSED, 0);
    assert!(zero.has(FLAG_DISCONTINUITY) && zero.has(FLAG_PAUSED));

    client.call("sim.step", json!({"n": 6})).unwrap();
    let (_, six) = frame_at_after(&mut client, 6, zero.seq, 0, FLAG_DISCONTINUITY);
    assert_eq!(six.tick, 6);
    assert!(!six.has(FLAG_DISCONTINUITY));

    client.call("sim.play", J::Null).unwrap();
    let deadline = Instant::now() + WAIT;
    let playing_head = loop {
        let state = client.call("sim.state", J::Null).unwrap();
        let tick = state["head_tick"].as_u64().unwrap();
        if tick > six.tick {
            break tick;
        }
        assert!(
            Instant::now() < deadline,
            "real-time play did not advance past tick {}",
            six.tick
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let paused = client.call("sim.pause", J::Null).unwrap();
    let paused_tick = paused["head_tick"].as_u64().unwrap();
    assert!(paused_tick >= playing_head);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        client.call("sim.state", J::Null).unwrap()["head_tick"],
        paused_tick
    );
    let (_, paused_frame) = frame_at_after(
        &mut client,
        paused_tick,
        six.seq,
        FLAG_PAUSED,
        FLAG_DISCONTINUITY,
    );

    client.call("sim.seek", json!({"tick": 3})).unwrap();
    let (_, seek) = frame_at_after(
        &mut client,
        3,
        paused_frame.seq,
        FLAG_DISCONTINUITY | FLAG_PAUSED,
        0,
    );
    assert_eq!(seek.tick, 3);
    assert!(seek.has(FLAG_DISCONTINUITY) && seek.has(FLAG_PAUSED));
    assert!(
        seek.entities.iter().all(|e| e.prev == e.cur),
        "a seek resets interpolation endpoints"
    );
    let stopped = client.call("sim.stop", json!({})).unwrap();
    assert_eq!(stopped["tick"], 3);
    assert_eq!(client.call("sim.state", J::Null).unwrap()["mode"], "edit");

    client.call("sim.start", json!({})).unwrap();
    let (_, restarted) = frame_at_after(
        &mut client,
        0,
        seek.seq,
        FLAG_DISCONTINUITY | FLAG_PAUSED,
        0,
    );
    assert_eq!(restarted.tick, 0);
    assert!(restarted.has(FLAG_DISCONTINUITY) && restarted.has(FLAG_PAUSED));
    assert!(restarted.entities.iter().all(|e| e.prev == e.cur));
}

struct TcpView {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl TcpView {
    fn connect(url: &str) -> TcpView {
        let addr = url
            .strip_prefix("ws://")
            .expect("loopback URL starts with ws://");
        let stream = TcpStream::connect(addr).expect("connect TCP ERP client");
        stream.set_read_timeout(Some(WAIT)).unwrap();
        TcpView {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    fn write_rpc(&mut self, value: J) {
        writeln!(self.writer, "{value}").unwrap();
    }

    fn call(&mut self, id: u64, method: &str, params: J) -> J {
        self.write_rpc(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        let deadline = Instant::now() + WAIT;
        loop {
            let message = self.next_json_until(deadline);
            if message["id"] == id {
                return message;
            }
            // A stream notification may be queued ahead of the RPC response.
        }
    }

    fn next_json_until(&mut self, deadline: Instant) -> J {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for a TCP ERP message"
        );
        self.reader
            .get_ref()
            .set_read_timeout(Some(remaining))
            .unwrap();
        let mut line = String::new();
        assert_ne!(
            self.reader.read_line(&mut line).expect("read TCP ERP line"),
            0,
            "TCP ERP server closed the connection"
        );
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("invalid TCP JSON {line:?}: {e}"))
    }

    fn subscribe(&mut self) -> J {
        self.write_rpc(json!({"jsonrpc":"2.0", "id":1, "method":"watch.subscribe", "params":{"topics":["viewstream"], "max_fps":1000}}));
        let deadline = Instant::now() + WAIT;
        let (mut response, mut schema) = (None, None);
        while response.is_none() || schema.is_none() {
            let message = self.next_json_until(deadline);
            if message["id"] == 1 {
                response = Some(message["result"].clone());
            } else if message["method"] == "watch.viewstream.schema" {
                schema = Some(message["params"].clone());
            }
        }
        let result = response.unwrap();
        assert_eq!(result["topics"], json!(["viewstream"]));
        schema.unwrap()
    }

    fn frame_at_after(
        &mut self,
        tick: u64,
        seq_after: u64,
        required_flags: u8,
        forbidden_flags: u8,
    ) -> (Vec<u8>, ViewFrame3) {
        let deadline = Instant::now() + WAIT;
        loop {
            let message = self.next_json_until(deadline);
            if message["method"] != "watch.viewstream" {
                continue;
            }
            assert_eq!(message["params"]["encoding"], "hex");
            let bytes = codec::hex_decode(message["params"]["data"].as_str().unwrap())
                .expect("valid stream hex");
            let frame = ViewFrame3::decode(&bytes).expect("decode TCP ViewFrame3");
            if frame.seq <= seq_after || frame.tick != tick {
                continue;
            }
            if frame.flags & required_flags != required_flags || frame.flags & forbidden_flags != 0
            {
                continue;
            }
            assert_eq!(frame.flags & required_flags, required_flags);
            assert_eq!(frame.flags & forbidden_flags, 0);
            return (bytes, frame);
        }
    }
}

#[test]
fn websocket_binary_and_tcp_hex_are_the_same_frame3_bytes() {
    let host = host();
    let url = host.url().unwrap().to_string();
    let mut ws = subscribed(&url);
    let ws_schema = wait_schema(&mut ws);
    let mut tcp = TcpView::connect(&url);
    let tcp_schema = tcp.subscribe();
    assert_eq!(ws_schema, tcp_schema);
    assert_eq!(tcp_schema["game"], "Yard3D");
    assert_eq!(tcp_schema["version"], 2);
    assert_eq!(tcp_schema["build_id"], format!("0x{YARD_BUILD_ID:016x}"));

    ws.call("sim.start", json!({})).unwrap();
    let (ws_initial, initial) = frame_at_after(&mut ws, 0, 0, FLAG_DISCONTINUITY | FLAG_PAUSED, 0);
    let (tcp_initial, tcp_initial_frame) =
        tcp.frame_at_after(0, 0, FLAG_DISCONTINUITY | FLAG_PAUSED, 0);
    assert_eq!(initial.tick, 0);
    assert_eq!(tcp_initial_frame.tick, 0);
    assert_eq!(ws_initial, tcp_initial);

    let malformed = tcp.call(2, "sim.input", json!({"player": 0, "input": "00"}));
    assert_eq!(malformed["error"]["code"], orr_remote::INVALID_PARAMS);
    assert!(
        malformed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("32 bytes")
    );
    let malformed_hex = tcp.call(3, "sim.input", json!({"player": 0, "input": "not-hex"}));
    assert_eq!(malformed_hex["error"]["code"], orr_remote::INVALID_PARAMS);
    let invalid_slot = tcp.call(
        4,
        "sim.input",
        json!({"player": YARD_PLAYERS, "input": input_hex(&input(0))}),
    );
    assert_eq!(invalid_slot["error"]["code"], orr_remote::INVALID_PARAMS);
    let malformed_command = tcp.call(5, "sim.command", json!({"player": 0, "command": "0000"}));
    assert_eq!(
        malformed_command["error"]["code"],
        orr_remote::INVALID_PARAMS
    );
    let structured_unavailable = tcp.call(6, "sim.input_value", json!({"player": 0, "value": {}}));
    assert_eq!(
        structured_unavailable["error"]["code"],
        orr_remote::INVALID_STATE
    );
    assert_eq!(
        structured_unavailable["error"]["data"]["kind"],
        "input_unavailable"
    );

    ws.call("sim.step", json!({"n": 11})).unwrap();
    let (ws_bytes, ws_frame) = frame_at_after(&mut ws, 11, initial.seq, 0, FLAG_DISCONTINUITY);
    let (tcp_bytes, tcp_frame) =
        tcp.frame_at_after(11, tcp_initial_frame.seq, 0, FLAG_DISCONTINUITY);
    assert_eq!(ws_frame.tick, 11);
    assert_eq!(tcp_frame.tick, 11);
    assert_frame_layout(&ws_bytes, &ws_frame);
    assert_eq!(
        ws_bytes, tcp_bytes,
        "WebSocket binary and TCP hex contain the same encoded 3D frame"
    );
}
