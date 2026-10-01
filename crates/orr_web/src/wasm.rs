//! The JavaScript-facing side (wasm32 only): `WebClient`.

use std::rc::Rc;

use orr_fp::FP;
use orr_proto::Channel;
use orr_session::{DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::PlayerSlot;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};
use wasm_bindgen::prelude::*;

use crate::{arena_bot_input, client_report_json, render_arena, LinkPort, WebLink, ARENA_BUILD_ID};

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

/// A relay client for the arena in the browser.
///
/// `opts` (a plain object): `room`, `build_id` (default the sample's),
/// `want_slot`, `bot` (play the scripted bot), plus what `js/transport.js`
/// reads: `mode`, `wtUrl`, `certHash`, `wsUrl`.
#[wasm_bindgen]
pub struct WebClient {
    client: RelayClient<Arena, WebLink>,
    transport: Rc<Transport>,
    port: LinkPort,
    _on_event: Closure<dyn FnMut(u8, u32, JsValue)>,
    bot: bool,
    ax: i32,
    ay: i32,
    fire: bool,
}

#[wasm_bindgen]
impl WebClient {
    #[wasm_bindgen(constructor)]
    pub fn new(opts: &JsValue) -> WebClient {
        let room = num(opts, "room").unwrap_or(1.0) as u64;
        let build_id = num(opts, "build_id").map_or(ARENA_BUILD_ID, |b| b as u64);
        let mut cfg = RelayClientConfig::new(room, build_id);
        cfg.want_slot = num(opts, "want_slot").map(|s| PlayerSlot(s as u8));

        let slot_port: Rc<std::cell::RefCell<Option<LinkPort>>> = Rc::default();
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
        let client = RelayClient::new(cfg, link, |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new());
        WebClient { client, transport, port, _on_event: on_event, bot: flag(opts, "bot"), ax: 0, ay: 0, fire: false }
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
        let t = self.transport.kind();
        client_report_json(&self.client, if t.is_empty() { "none" } else { &t })
    }

    /// Four integers per entity, see `render_arena`.
    pub fn render(&self) -> Vec<i32> {
        render_arena(&self.client)
    }

    /// The transport in use: `webtransport`, `websocket`, or empty while connecting.
    pub fn transport(&self) -> String {
        self.transport.kind()
    }

    /// Why the transport failed (empty when it did not).
    pub fn transport_error(&self) -> String {
        self.transport.error()
    }

    /// Datagram/stream messages this link moved to the reliable channel for their size.
    pub fn oversize_to_reliable(&self) -> f64 {
        self.port.oversize_to_reliable() as f64
    }

    /// Tells the server this client leaves, then closes the transport.
    pub fn leave(&mut self) {
        self.client.leave();
    }

    pub fn close(&self) {
        self.transport.close();
    }
}
