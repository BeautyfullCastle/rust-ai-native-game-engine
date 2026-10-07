//! Test-only Arena host. Same seed and two-player layout as the normal-input CLI proof.
#![allow(dead_code)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

use orr_edit::{EditorDoc, PlayController};
use orr_remote::{Auth, ErpClient, ErpServer, GameHooks, Host, ServerConfig};
use orr_sample::arena_game::{Arena, ArenaConfig, ArenaMetrics};
use orr_session::{ControlOp, PlaySession};
use orr_sim::Simulation;
use serde_json::{json, Value as J};

pub const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [-300,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    name: target\n    Position: { pos: [300,0] }\n    PlayerTag: { slot: 1 }\n";

pub struct ArenaHost {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ArenaHost {
    pub fn start(viewer: bool) -> Self {
        Self::named(viewer, "Arena")
    }

    pub fn named(viewer: bool, game: &'static str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut types = orr_reflect::TypeRegistry::new();
            orr_sample::arena_game::register_reflect(&mut types);
            let doc = EditorDoc::from_yaml(SCENE, types, Simulation::<Arena>::build_registry(), 42)
                .unwrap();
            let mut cfg = ServerConfig::new(Auth::DevNoAuth);
            cfg.bind.set_port(0);
            cfg.limits.allow_scene_paths = true;
            cfg.limits.build_id = orr_remote::default_build_id(game);
            cfg.limits.game = GameHooks::new(game).with_metrics(ArenaMetrics);
            let mut server = ErpServer::start(cfg).unwrap();
            server.set_structured_input::<Arena>("ArenaInput", 8, |slot, input| {
                orr_sample::arena_view::arena_fire_commands(u32::from(slot.0), input)
            });
            server.enable_managed_input();
            let mut host = Host::<Arena>::new(doc, server);
            if viewer {
                let mut play =
                    PlayController::<Arena>::start_play(&host.doc, host.doc.play_config(2, 60))
                        .unwrap();
                play.control(ControlOp::Step(30));
                let replay = play.session().save_replay();
                *play.session_mut() =
                    PlaySession::open_replay(&replay, ArenaConfig { player_count: 2 }, 0).unwrap();
                host.play = Some(play);
            }
            tx.send(host.server.url()).unwrap();
            host.run(&stopped, Duration::from_micros(500));
        });
        Self {
            url: rx.recv_timeout(Duration::from_secs(20)).unwrap(),
            stop,
            thread: Some(thread),
        }
    }
    pub fn client(&self) -> ErpClient {
        ErpClient::connect(&self.url, None).unwrap()
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
impl Drop for ArenaHost {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn input(client: &mut ErpClient, x: i32, fire: bool) {
    client.call("sim.input_value", json!({"player":0,"value":{"axis_x":x,"axis_y":0,"buttons":if fire {vec!["fire"]} else {vec![]}}})).unwrap();
}

pub fn proposal(client: &mut ErpClient, x: i32, y: i32) -> String {
    let id = client
        .call("proposal.begin", json!({"label":"Arena position preview"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    client.call("proposal.apply", json!({"id":id,"ops":[{"op":"patch","entity":"e_00000001","component":"Position","path":"pos","value":[x,y]}]})).unwrap();
    id
}

pub fn position(client: &mut ErpClient) -> J {
    client
        .call(
            "world.get",
            json!({"entity":"e_00000001","component":"Position","path":"pos"}),
        )
        .unwrap()["value"]
        .clone()
}
