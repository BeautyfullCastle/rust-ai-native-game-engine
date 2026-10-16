//! Real ERP proof of normal Arena input, raw compatibility, reflection,
//! command recording and same-input scene verification.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

use orr_edit::EditorDoc;
use orr_fp::FP;
use orr_reflect::TypeRegistry;
use orr_remote::{Auth, Caps, ErpClient, ErpServer, GameHooks, Host, ServerConfig, TokenEntry};
use orr_session::ReplayReader;
use orr_sim::{PlayerSlot, Simulation};
use orr_testgame::{Arena, ArenaInput, ArenaMetrics, SpawnBulletCmd, FIRE};
use serde_json::{json, Value as J};

const BUILD: u64 = 12345;
const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [-300,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    name: target\n    Position: { pos: [300,0] }\n    PlayerTag: { slot: 1 }\n";

struct ArenaHost {
    url: String,
    connector: orr_remote::LocalConnector,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ArenaHost {
    fn start(adapter: bool) -> Self {
        Self::start_managed(adapter, false)
    }
    fn start_managed(adapter: bool, managed: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut types = TypeRegistry::new();
            orr_testgame::register_reflect(&mut types);
            let doc = EditorDoc::from_yaml(SCENE, types, Simulation::<Arena>::build_registry(), 42)
                .unwrap();
            let mut cfg = ServerConfig::new(Auth::Tokens(vec![
                TokenEntry {
                    client: "all".into(),
                    token: "all".into(),
                    caps: Caps::ALL,
                },
                TokenEntry {
                    client: "read".into(),
                    token: "read".into(),
                    caps: Caps::parse("read").unwrap(),
                },
            ]));
            cfg.bind.set_port(0);
            cfg.limits.build_id = BUILD;
            cfg.limits.game = GameHooks::new("Arena").with_metrics(ArenaMetrics);
            let mut server = ErpServer::start(cfg).unwrap();
            if adapter {
                server.set_structured_input::<Arena>("ArenaInput", 8, |slot, input| {
                    if input.buttons & FIRE != 0 {
                        vec![SpawnBulletCmd {
                            owner: u32::from(slot.0),
                        }]
                    } else {
                        vec![]
                    }
                });
            }
            if managed {
                server.enable_managed_input();
            }
            tx.send((server.url(), server.connector())).unwrap();
            Host::<Arena>::new(doc, server).run(&stopped, Duration::from_micros(500));
        });
        let (url, connector) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        Self {
            url,
            connector,
            stop,
            thread: Some(thread),
        }
    }
    fn client(&self, token: &str) -> ErpClient {
        ErpClient::connect(&self.url, Some(token)).unwrap()
    }
}
impl Drop for ArenaHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn input(c: &mut ErpClient, slot: u8, x: i32, fire: bool) {
    c.call("sim.input_value", json!({"player":slot,"value":{"axis_x":x,"axis_y":0,"buttons":if fire { vec!["fire"] } else { vec![] }}})).unwrap();
}
fn score(c: &mut ErpClient, slot: u8) -> J {
    c.call(
        "world.singleton.get",
        json!({"name":"Score","path":format!("kills[{slot}]")}),
    )
    .unwrap()["value"]
        .clone()
}
fn replay(c: &mut ErpClient) -> Vec<u8> {
    let stopped = c.call("sim.stop", json!({"include_replay":true})).unwrap();
    orr_remote::codec::b64_decode(stopped["replay"].as_str().unwrap()).unwrap()
}

#[test]
fn reflected_normal_inputs_move_fire_and_record_exactly() {
    let host = ArenaHost::start(true);
    let mut c = host.client("all");
    let discover = c.call("rpc.discover", json!({})).unwrap();
    assert_eq!(discover["engine"]["game"], "Arena");
    assert_eq!(discover["engine"]["verify"]["bot_available"], false);
    let schema = c.call("registry.input", json!({})).unwrap();
    assert_eq!(schema["schema"]["title"], "ArenaInput");
    assert_eq!(schema, discover["engine"]["input"]);
    assert!(schema["schema"]["properties"].get("_pad").is_none());
    assert_eq!(schema["schema"]["properties"]["axis_x"]["maximum"], 1);
    assert_eq!(schema["max_players"], 8);
    assert!(c.call("sim.start", json!({"player_count":9})).is_err());
    let initial = c.call("sim.checksum", json!({})).unwrap();
    c.call("sim.start", json!({})).unwrap();
    input(&mut c, 0, 1, false);
    c.call("sim.step", json!({"n":10})).unwrap();
    let moved = c
        .call(
            "world.get",
            json!({"entity":"e_00000001","component":"Position","path":"pos"}),
        )
        .unwrap();
    assert_eq!(moved["value"], json!([-240, 0]));
    input(&mut c, 0, 0, false);
    c.call("sim.step", json!({"n":5})).unwrap();
    assert_eq!(
        c.call(
            "world.get",
            json!({"entity":"e_00000001","component":"Position","path":"pos"})
        )
        .unwrap(),
        moved
    );
    replay(&mut c);
    assert_eq!(c.call("sim.checksum", json!({})).unwrap(), initial);
    c.call("sim.start", json!({})).unwrap();
    input(&mut c, 0, 0, true);
    c.call("sim.step", json!({"n":1})).unwrap();
    input(&mut c, 0, 0, false);
    c.call("sim.step", json!({"n":19})).unwrap();
    assert_eq!(score(&mut c, 0), 1);
    assert_eq!(score(&mut c, 1), 0);
    let bytes = replay(&mut c);
    let reader = ReplayReader::<Arena>::parse(&bytes).unwrap();
    assert_eq!(reader.header.game_id, "Arena");
    assert_eq!(reader.header.build_hash, orr_sim::build_hash_of(BUILD, 0));
    assert_eq!(reader.header.seed, 42);
    assert_eq!(
        reader.tick(1).unwrap().1,
        vec![(PlayerSlot(0), SpawnBulletCmd { owner: 0 })]
    );
    for tick in 2..=20 {
        assert!(reader.tick(tick).unwrap().1.is_empty());
    }
    let report = c.call("verify.self", json!({"inputs":{"kind":"last_play"},"checks":["recording_matches","score_0.final == 1","score_1.final == 0","bullets.final == 0","players.final == 2"]})).unwrap();
    assert_eq!(report["passed"], true, "{report}");
    assert_eq!(report["recording"]["checked"], 21);
    assert_eq!(report["recording"]["mismatches"], 0);
    // A normal shot exists between the two default samples, then scores and
    // disappears before the final tick. Sparse max is intentionally sampled.
    let bullet = |report: &J| {
        report["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == "bullets")
            .unwrap()["base"]["max"]
            .clone()
    };
    assert_eq!(bullet(&report), 0);
    assert_eq!(
        report["metric_sampling"],
        json!({
            "requested_interval": 60, "sample_count": 2, "scope": "sampled_tick_boundaries", "every_tick_boundary_observed": false
        })
    );
    let dense = c.call("verify.self", json!({"inputs":{"kind":"last_play"},"sample_every":1,"checks":["recording_matches","score_0.final == 1","score_1.final == 0","bullets.final == 0","players.final == 2","bullets.max == 1"]})).unwrap();
    assert_eq!(dense["passed"], true, "{dense}");
    assert_eq!(bullet(&dense), 1);
    assert_eq!(
        dense["metric_sampling"],
        json!({
            "requested_interval": 1, "sample_count": 21, "scope": "sampled_tick_boundaries", "every_tick_boundary_observed": true
        })
    );
    assert_eq!(dense["checksums"], report["checksums"]);
    assert_eq!(dense["recording"], report["recording"]);
    assert_eq!(dense["identical"], report["identical"]);
    assert_eq!(dense["first_divergence"], report["first_divergence"]);
}

#[test]
fn structured_values_validate_before_replacing_held_input_and_require_permission() {
    let host = ArenaHost::start(true);
    let mut c = host.client("all");
    let mut read = host.client("read");
    assert!(read.call("registry.input", json!({})).is_ok());
    c.call("sim.start", json!({})).unwrap();
    input(&mut c, 0, 1, false);
    for bad in [
        json!({"axis_x":2,"axis_y":0,"buttons":[]}),
        json!({"axis_x":0,"axis_y":0,"buttons":["unknown"]}),
        json!({"axis_x":0}),
        json!({"axis_x":0,"axis_y":0,"buttons":[],"_pad":1}),
    ] {
        assert!(c
            .call("sim.input_value", json!({"player":0,"value":bad}))
            .is_err());
    }
    assert!(c
        .call(
            "sim.input_value",
            json!({"player":2,"value":{"axis_x":0,"axis_y":0,"buttons":[]}})
        )
        .is_err());
    assert_eq!(
        read.call_err(
            "sim.input_value",
            json!({"player":0,"value":{"axis_x":0,"axis_y":0,"buttons":[]}})
        )
        .data
        .unwrap()["kind"],
        "permission_denied"
    );
    c.call("sim.step", json!({"n":1})).unwrap();
    assert_eq!(
        c.call(
            "world.get",
            json!({"entity":"e_00000001","component":"Position","path":"pos"})
        )
        .unwrap()["value"],
        json!([-294, 0])
    );
}

#[test]
fn raw_input_preserves_explicit_command_semantics_and_slot_ownership() {
    let host = ArenaHost::start(true);
    let mut c = host.client("all");
    c.call("sim.start", json!({})).unwrap();
    let fire = ArenaInput::new(FP::ZERO, FP::ZERO, true);
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    input(&mut c, 0, 0, true);
    c.call(
        "sim.input",
        json!({"player":0,"input":hex(bytemuck::bytes_of(&fire))}),
    )
    .unwrap();
    c.call(
        "sim.command",
        json!({"player":0,"command":hex(bytemuck::bytes_of(&SpawnBulletCmd { owner: 0 }))}),
    )
    .unwrap();
    input(&mut c, 1, 0, true);
    c.call("sim.step", json!({"n":2})).unwrap();
    input(&mut c, 0, 0, true);
    input(&mut c, 1, 0, false);
    c.call("sim.step", json!({"n":1})).unwrap();
    let bytes = replay(&mut c);
    let reader = ReplayReader::<Arena>::parse(&bytes).unwrap();
    assert_eq!(
        reader.tick(1).unwrap().1,
        vec![
            (PlayerSlot(0), SpawnBulletCmd { owner: 0 }),
            (PlayerSlot(1), SpawnBulletCmd { owner: 1 })
        ]
    );
    assert_eq!(
        reader.tick(2).unwrap().1,
        vec![(PlayerSlot(1), SpawnBulletCmd { owner: 1 })]
    );
    assert_eq!(
        reader.tick(3).unwrap().1,
        vec![(PlayerSlot(0), SpawnBulletCmd { owner: 0 })]
    );
}

#[test]
fn structured_input_is_unavailable_without_opt_in() {
    let host = ArenaHost::start(false);
    let mut c = host.client("all");
    assert!(c.call("rpc.discover", json!({})).unwrap()["engine"]
        .get("input")
        .is_none());
    assert_eq!(
        c.call_err("registry.input", json!({})).data.unwrap()["kind"],
        "input_unavailable"
    );
    assert_eq!(
        c.call_err("sim.input_value", json!({"player":0,"value":{}}))
            .data
            .unwrap()["kind"],
        "input_unavailable"
    );
}

#[test]
fn managed_transports_preserve_normal_arena_recording() {
    for local in [false, true] {
        managed_transport_oracle(local);
    }
}

// Host-side disconnect timeout, never simulation state.
#[allow(clippy::disallowed_types)]
fn managed_transport_oracle(local: bool) {
    let host = ArenaHost::start_managed(true, true);
    let connect = |token| {
        if local {
            let caps = if token == "read" {
                Caps::parse("read").unwrap()
            } else {
                Caps::ALL
            };
            ErpClient::with_transport(Box::new(host.connector.connect(token, caps).unwrap()))
        } else {
            host.client(token)
        }
    };
    let mut c = connect("all");
    let mut other = connect("all"); // Identical labels do not convey ownership.
    let mut read = connect("read");
    let denied = read
        .call("sim.input_claim", json!({"player":0,"replace_held":true}))
        .unwrap_err();
    assert_eq!(
        denied.rpc().unwrap().data.as_ref().unwrap()["kind"],
        "permission_denied"
    );
    assert_eq!(
        c.call("registry.input", json!({})).unwrap()["managed_held"]["version"],
        1
    );
    c.call("sim.start", json!({})).unwrap();
    input(&mut c, 0, 1, true);
    let grant = c
        .call("sim.input_claim", json!({"player":0,"replace_held":true}))
        .unwrap();
    let tagged = |sequence: u64, fire: bool| json!({"player":0,"grant":grant["grant"],"generation":grant["generation"],"sequence":sequence.to_string(),"value":{"axis_x":0,"axis_y":0,"buttons":if fire {vec!["fire"]} else {vec![]}}});
    assert!(other
        .call("sim.input_claim", json!({"player":0,"replace_held":true}))
        .is_err());
    assert!(other.call("sim.input_value", tagged(1, true)).is_err());
    assert!(c
        .call(
            "sim.input_value",
            json!({"player":0,"value":{"axis_x":1,"axis_y":0,"buttons":[]}})
        )
        .is_err());
    c.call("sim.input_value", tagged(1, true)).unwrap();
    c.call("sim.step", json!({"n":1})).unwrap();
    c.call("sim.input_release", tagged(2, false)).unwrap();
    c.call("sim.step", json!({"n":19})).unwrap();
    assert_eq!(score(&mut c, 0), 1);
    let bytes = replay(&mut c);
    let reader = ReplayReader::<Arena>::parse(&bytes).unwrap();
    assert_eq!(
        reader.tick(1).unwrap().1,
        vec![(PlayerSlot(0), SpawnBulletCmd { owner: 0 })]
    );
    for tick in 2..=20 {
        assert!(reader.tick(tick).unwrap().1.is_empty());
    }
    let report = c.call("verify.self", json!({"inputs":{"kind":"last_play"},"checks":["recording_matches","score_0.final == 1"]})).unwrap();
    assert_eq!(report["passed"], true, "{report}");
    assert_eq!(report["recording"]["mismatches"], 0);
    c.call("sim.start", json!({})).unwrap();
    assert!(c.call("sim.input_value", tagged(3, true)).is_err());
    for (method, params) in [
        ("sim.pause", json!({})),
        ("sim.seek", json!({"tick":0})),
        ("sim.branch", json!({})),
    ] {
        let g = c
            .call("sim.input_claim", json!({"player":0,"replace_held":true}))
            .unwrap();
        let status = c.call("sim.state", json!({})).unwrap();
        assert_eq!(status["managed_held"]["slots"][0]["grant"], g["grant"]);
        assert_eq!(status["managed_held"]["slots"][0]["owned_by_you"], true);
        assert_eq!(
            other.call("sim.state", json!({})).unwrap()["managed_held"]["slots"][0]["owned_by_you"],
            false
        );
        c.call(method, params).unwrap();
        assert_eq!(
            c.call("sim.state", json!({})).unwrap()["managed_held"]["slots"],
            json!([])
        );
        assert!(c
            .call(
                "sim.input_renew",
                json!({"player":0,"grant":g["grant"],"generation":g["generation"],"sequence":"1"})
            )
            .is_err());
    }
    other
        .call("sim.input_claim", json!({"player":0,"replace_held":true}))
        .unwrap();
    drop(other);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !c.call("sim.state", json!({})).unwrap()["managed_held"]["slots"]
        .as_array()
        .unwrap()
        .is_empty()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "disconnected owner did not clear"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
