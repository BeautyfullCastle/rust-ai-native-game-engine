//! The JavaScript-facing side (wasm32 only): `WebClient` (arena) and `PhysClient` (physics).
//!
//! Both are meant to live in a Web Worker (`web/worker.js`): the transport
//! (`js/transport.js`, WebTransport or WebSocket) works there, and a worker's
//! timers are not throttled when the tab is in the background.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use orr_fp::FP;
use orr_games::physics_game::{bot_input, NoCommand, PhysConfig, PhysInput};
use orr_proto::Channel;
use orr_session::{DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::PlayerSlot;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};
use wasm_bindgen::prelude::*;

use crate::{
    arena_bot_input, client_report_json, render_arena, render_phys, scene_box, LinkPort, WebLink, ARENA_BUILD_ID,
    PHYSICS_BUILD_ID,
};

#[wasm_bindgen(module = "/js/transport.js")]
extern "C" {
    /// WebTransport with a WebSocket fallback, see `js/transport.js`.
    type Transport;

    #[wasm_bindgen(constructor)]
    fn new(opts: &JsValue, on_event: &js_sys::Function) -> Transport;
    #[wasm_bindgen(method)]
    fn send(this: &Transport, channel: u8, data: &[u8]);
    #[wasm_bindgen(method)]
    fn close(this: &Transport);
    #[wasm_bindgen(method, getter)]
    fn kind(this: &Transport) -> String;
    #[wasm_bindgen(method, getter)]
    fn error(this: &Transport) -> String;
}

fn num(opts: &JsValue, key: &str) -> Option<f64> {
    js_sys::Reflect::get(opts, &JsValue::from_str(key)).ok().and_then(|v| v.as_f64())
}

fn flag(opts: &JsValue, key: &str) -> bool {
    js_sys::Reflect::get(opts, &JsValue::from_str(key)).ok().is_some_and(|v| v.is_truthy())
}

/// The browser transport and the callback that feeds it into a [`WebLink`].
struct Wire {
    transport: Rc<Transport>,
    port: LinkPort,
    _on_event: Closure<dyn FnMut(u8, u32, JsValue)>,
}

impl Wire {
    fn new(opts: &JsValue) -> (WebLink, Wire) {
        let slot_port: Rc<RefCell<Option<LinkPort>>> = Rc::default();
        let cb_port = slot_port.clone();
        let on_event = Closure::<dyn FnMut(u8, u32, JsValue)>::new(move |kind: u8, a: u32, data: JsValue| {
            let borrow = cb_port.borrow();
            let Some(port) = borrow.as_ref() else { return };
            match kind {
                0 => port.connected(a as usize),
                1 => port.disconnected(),
                _ => {
                    let bytes = js_sys::Uint8Array::new(&data).to_vec();
                    port.message(if a == 0 { Channel::Reliable } else { Channel::Unreliable }, bytes);
                }
            }
        });
        let transport = Rc::new(Transport::new(opts, on_event.as_ref().unchecked_ref()));
        let (t_send, t_close) = (transport.clone(), transport.clone());
        let (link, port) = WebLink::new(
            move |channel, data| t_send.send(if channel == Channel::Reliable { 0 } else { 1 }, data),
            move || t_close.close(),
        );
        *slot_port.borrow_mut() = Some(port.clone());
        (link, Wire { transport, port, _on_event: on_event })
    }

    fn kind(&self) -> String {
        let t = self.transport.kind();
        if t.is_empty() {
            "none".to_string()
        } else {
            t
        }
    }
}

fn client_config(opts: &JsValue, default_build: u64) -> RelayClientConfig {
    let room = num(opts, "room").unwrap_or(1.0) as u64;
    let build_id = num(opts, "build_id").map_or(default_build, |b| b as u64);
    let mut cfg = RelayClientConfig::new(room, build_id);
    cfg.want_slot = num(opts, "want_slot").map(|s| PlayerSlot(s as u8));
    cfg
}

/// A relay client for the arena in the browser.
///
/// `opts` (a plain object): `room`, `build_id` (default the sample's),
/// `want_slot`, `bot` (play the scripted bot), plus what `js/transport.js`
/// reads: `mode`, `wtUrl`, `certHash`, `wsUrl`.
#[wasm_bindgen]
pub struct WebClient {
    client: RelayClient<Arena, WebLink>,
    wire: Wire,
    bot: bool,
    ax: i32,
    ay: i32,
    fire: bool,
}

#[wasm_bindgen]
impl WebClient {
    #[wasm_bindgen(constructor)]
    pub fn new(opts: &JsValue) -> WebClient {
        let cfg = client_config(opts, ARENA_BUILD_ID);
        let (link, wire) = Wire::new(opts);
        let client = RelayClient::new(cfg, link, |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new());
        WebClient { client, wire, bot: flag(opts, "bot"), ax: 0, ay: 0, fire: false }
    }

    /// One step of the client at `now_us` (microseconds, any monotonic clock).
    pub fn tick(&mut self, now_us: f64) {
        let slot = self.client.welcome().map_or(0, |w| w.slot);
        let (bot, ax, ay, fire) = (self.bot, self.ax, self.ay, self.fire);
        self.client.update(now_us as u64, &mut |tick| {
            if bot {
                arena_bot_input(slot, tick)
            } else {
                let input = ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire);
                let cmds = if input.buttons & FIRE != 0 { vec![SpawnBulletCmd { owner: u32::from(slot) }] } else { Vec::new() };
                (input, cmds)
            }
        });
    }

    /// The local player's input (axes -1, 0, 1, fire) for the ticks to come.
    pub fn set_input(&mut self, ax: i32, ay: i32, fire: bool) {
        self.ax = ax.clamp(-1, 1);
        self.ay = ay.clamp(-1, 1);
        self.fire = fire;
    }

    pub fn set_bot(&mut self, on: bool) {
        self.bot = on;
    }

    /// JSON report: state, transport, rollbacks, desyncs, verified checksums.
    pub fn report(&self) -> String {
        client_report_json(&self.client, &self.wire.kind())
    }

    /// Four integers per entity, see `render_arena`.
    pub fn render(&self) -> Vec<i32> {
        render_arena(&self.client)
    }

    /// The transport in use: `webtransport`, `websocket`, or empty while connecting.
    pub fn transport(&self) -> String {
        self.wire.transport.kind()
    }

    /// Why the transport failed (empty when it did not).
    pub fn transport_error(&self) -> String {
        self.wire.transport.error()
    }

    /// Datagram/stream messages this link moved to the reliable channel for their size.
    pub fn oversize_to_reliable(&self) -> f64 {
        self.wire.port.oversize_to_reliable() as f64
    }

    /// Tells the server this client leaves, then closes the transport.
    pub fn leave(&mut self) {
        self.client.leave();
    }

    pub fn close(&self) {
        self.wire.transport.close();
    }
}

/// A relay client for the physics sample (`orr_games::physics_game::PhysGame`).
/// The scene comes from the room's config blob (`orr_server --game physics`).
/// `opts` as for [`WebClient`], with the physics build id as default.
#[wasm_bindgen]
pub struct PhysClient {
    client: RelayClient<orr_games::physics_game::PhysGame, WebLink>,
    wire: Wire,
    scene: Rc<Cell<Option<PhysConfig>>>,
    bot: bool,
    bot_seed: u64,
    input: PhysInput,
}

#[wasm_bindgen]
impl PhysClient {
    #[wasm_bindgen(constructor)]
    pub fn new(opts: &JsValue) -> PhysClient {
        let cfg = client_config(opts, PHYSICS_BUILD_ID);
        let (link, wire) = Wire::new(opts);
        let scene: Rc<Cell<Option<PhysConfig>>> = Rc::default();
        let sink = scene.clone();
        let client = RelayClient::new(
            cfg,
            link,
            move |w| {
                let scene = PhysConfig::from_blob(&w.config, w.player_count)
                    .expect("the room config is not a physics scene (start the server with --game physics)");
                sink.set(Some(scene));
                scene
            },
            DumpCollector::new(),
        );
        PhysClient {
            client,
            wire,
            scene,
            bot: flag(opts, "bot"),
            bot_seed: num(opts, "bot_seed").map_or(1234, |s| s as u64),
            input: PhysInput::default(),
        }
    }

    /// One step of the client at `now_us` (microseconds, any monotonic clock).
    pub fn tick(&mut self, now_us: f64) {
        let slot = self.client.welcome().map_or(0, |w| w.slot);
        let (bot, seed, input) = (self.bot, self.bot_seed, self.input);
        self.client.update(now_us as u64, &mut |tick| {
            let i = if bot { bot_input(seed, tick, PlayerSlot(slot)) } else { input };
            (i, Vec::<NoCommand>::new())
        });
    }

    /// The local paddle: stick axes and spin (-1, 0, 1) and the shoot button.
    pub fn set_input(&mut self, ax: i32, ay: i32, spin: i32, shoot: bool) {
        self.input = PhysInput::new(ax.clamp(-1, 1), ay.clamp(-1, 1), spin.clamp(-1, 1), shoot);
    }

    pub fn set_bot(&mut self, on: bool) {
        self.bot = on;
    }

    pub fn report(&self) -> String {
        client_report_json(&self.client, &self.wire.kind())
    }

    /// Eight integers per body, see `render_phys`. Empty until the room started.
    pub fn render(&self) -> Vec<i32> {
        self.client.session().map_or_else(Vec::new, |s| render_phys(s.predicted_frame()))
    }

    /// `[half width, height]` of the scene box in world units, empty until the room started.
    pub fn scene_box(&self) -> Vec<i32> {
        self.scene.get().map_or_else(Vec::new, |c| scene_box(&c).to_vec())
    }

    pub fn transport(&self) -> String {
        self.wire.transport.kind()
    }

    pub fn transport_error(&self) -> String {
        self.wire.transport.error()
    }

    pub fn oversize_to_reliable(&self) -> f64 {
        self.wire.port.oversize_to_reliable() as f64
    }

    pub fn leave(&mut self) {
        self.client.leave();
    }

    pub fn close(&self) {
        self.wire.transport.close();
    }
}
