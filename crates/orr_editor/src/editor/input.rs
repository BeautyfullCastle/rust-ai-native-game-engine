//! Explicit, bounded, nonblocking ownership of one realtime Arena input slot.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys { pub x: i8, pub y: i8, pub fire: bool }
impl Keys {
    fn value(self) -> J { json!({"axis_x":self.x,"axis_y":self.y,"buttons":if self.fire {vec!["fire"]} else {vec![]}}) }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase { #[default] Off, Claiming, Active, Releasing }
#[derive(Clone, Debug)]
struct Grant { player: u8, grant: String, generation: String, sequence: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op { Claim, Value, Renew, Release }
#[derive(Default)]
pub(super) struct Input {
    pub phase: Phase,
    pub enabled: bool,
    intent: u64,
    wanted: bool,
    player: u8,
    grant: Option<Grant>,
    latest: Keys,
    sent: Option<Keys>,
    flight: Option<(u64, Op, Instant)>,
    last_ack: Option<Instant>,
}
impl Input {
    pub fn new(enabled: bool) -> Self { Self { enabled, ..Default::default() } }
    pub fn claim(&mut self, player: u8) {
        if self.phase != Phase::Off { return; }
        self.intent += 1;
        self.player = player;
        self.wanted = true;
        self.phase = Phase::Claiming;
        self.latest = Keys::default();
        self.sent = None;
    }
    pub fn release(&mut self) {
        self.wanted = false;
        self.latest = Keys::default();
        if self.phase != Phase::Off { self.phase = Phase::Releasing; }
    }
    pub fn keys(&mut self, keys: Keys) { if self.wanted { self.latest = keys; } }
    fn off(&mut self) {
        self.phase = Phase::Off; self.wanted = false; self.grant = None;
        self.flight = None; self.sent = None; self.last_ack = None;
    }
    pub fn next(&mut self, now: Instant) -> Option<(&'static str, J, u64, Op)> {
        if let Some((_, op, at)) = self.flight {
            if now.duration_since(at) >= Duration::from_millis(2000) {
                self.wanted = false;
                self.latest = Keys::default();
                if matches!(op, Op::Claim | Op::Release) {
                    // Keep the request fence until its reply or reconnect. A
                    // delayed claim must still be cleaned up, never rearmed.
                    self.phase = Phase::Off;
                    return None;
                }
                self.phase = Phase::Releasing;
            }
            // Reserve one cleanup message even when an update/renew reply is
            // delayed. Ordered ERP sends and increasing sequences fence it.
            // The obsolete acknowledgement cannot rearm this machine.
            if self.phase == Phase::Releasing && self.grant.is_some() && matches!(op, Op::Value | Op::Renew) {
                self.flight = None;
            } else {
                return None;
            }
        }
        let op = match self.phase {
            Phase::Off => return None,
            Phase::Claiming => Op::Claim,
            Phase::Releasing => if self.grant.is_some() { Op::Release } else { self.off(); return None; },
            Phase::Active => {
                if self.last_ack.is_some_and(|at| now.duration_since(at) >= Duration::from_millis(2000)) { self.off(); return None; }
                if self.sent != Some(self.latest) { Op::Value }
                else if self.last_ack.is_some_and(|at| now.duration_since(at) >= Duration::from_millis(500)) { Op::Renew }
                else { return None; }
            }
        };
        let (method, params) = if op == Op::Claim {
            ("sim.input_claim", json!({"player":self.player,"replace_held":true}))
        } else {
            let g = self.grant.as_mut()?;
            g.sequence = g.sequence.checked_add(1)?;
            let mut params = json!({"player":g.player,"grant":g.grant,"generation":g.generation,"sequence":g.sequence.to_string()});
            let method = match op {
                Op::Value => { params["value"] = self.latest.value(); self.sent = Some(self.latest); "sim.input_value" },
                Op::Renew => "sim.input_renew",
                Op::Release => "sim.input_release",
                Op::Claim => unreachable!(),
            };
            (method, params)
        };
        self.flight = Some((self.intent, op, now));
        Some((method, params, self.intent, op))
    }
    pub fn answer(&mut self, intent: u64, op: Op, result: Result<J, orr_remote::RpcError>, now: Instant) -> Option<String> {
        if self.flight.map(|(i,o,_)| (i,o)) != Some((intent,op)) { return None; }
        let sent_at = self.flight.expect("matched request").2;
        self.flight = None;
        if now.duration_since(sent_at) >= Duration::from_millis(2000) {
            self.wanted = false;
            if op != Op::Claim {
                self.off();
                return Some("Arena control: acknowledgement exceeded lease deadline; control is off".into());
            }
            // A late successful claim still carries the grant needed to send
            // cleanup, even though it must never send a gameplay value.
        }
        let value = match result { Ok(v) => v, Err(e) => { self.off(); return Some(format!("Arena control is off; release unconfirmed (host lease expiry is the fallback): {}",e.message)); } };
        let valid = value["accepted_head_tick"].as_u64().is_some();
        if op == Op::Claim {
            let strings = value["grant"].as_str().zip(value["generation"].as_str());
            if let Some((grant,generation)) = strings.filter(|(g, generation)| g.parse::<u64>().is_ok() && generation.parse::<u64>().is_ok() && valid && value["player"].as_u64() == Some(u64::from(self.player)) && value["lease_ms"].as_u64() == Some(2000)) {
                self.grant = Some(Grant { player:self.player, grant:grant.into(), generation:generation.into(), sequence:0 });
                self.phase = if self.wanted { Phase::Active } else { Phase::Releasing };
            } else { self.off(); return Some("Arena control: invalid claim acknowledgement; lease will expire".into()); }
        } else if !valid || !self.grant.as_ref().is_some_and(|g| value["ok"] == true && value["player"] == g.player && value["grant"] == g.grant && value["generation"] == g.generation && value["sequence"].as_str().and_then(|s| s.parse::<u64>().ok()) == Some(g.sequence)) {
            self.off(); return Some("Arena control: missing admission acknowledgement; lease will expire".into());
        } else if op == Op::Release { self.off(); }
        self.last_ack = Some(sent_at);
        None
    }
}

impl Editor {
    /// The optional Arena keyboard ownership state. Focus never acquires it.
    pub fn input_phase(&self) -> Phase { self.input.phase }
    /// Why keyboard control is off, including unavailable capability negotiation.
    pub fn input_hint(&self) -> &'static str {
        if self.input_cleanup_pending() { "Off · cleanup reply pending; lease expiry is the fallback" }
        else if !self.input.enabled { "Off · compatible managed input unavailable" }
        else if self.is_viewer() { "Off · replay Viewer is read-only" }
        else if self.preview.is_some() { "Off · proposal preview" }
        else if !self.sim.playing { "Off · start realtime Play to take control" }
        else if self.sim.head_tick != self.sim.last_tick { "Off · wait for live head" }
        else { "Off · explicit Take control required" }
    }
    pub fn input_cleanup_pending(&self) -> bool {
        self.input.phase == Phase::Off && self.pending.values().any(|p| matches!(p.kind, Pend::Input(..)))
    }
    pub fn can_take_control(&self) -> bool {
        self.game() == EditorGame::Arena && !self.input_cleanup_pending() && self.input.enabled && self.down.is_none()
            && !self.is_viewer() && self.preview.is_none() && self.sim.mode == Mode::Play
            && self.sim.playing && self.sim.head_tick == self.sim.last_tick
    }
    /// Explicit human intent only; the caller must deliberately focus the viewport.
    pub fn take_control(&mut self, player: u8, viewport_focused: bool) -> bool {
        if !viewport_focused || !self.can_take_control() || player >= self.sim.player_count || self.input.phase != Phase::Off || self.pending.values().any(|p| matches!(p.kind, Pend::Input(..))) { return false; }
        self.state_generation += 1;
        self.input.claim(player); self.pump_input(); true
    }
    pub fn release_control(&mut self) { self.input.release(); self.pump_input(); }
    /// UI focus/capture gate. Losing focus is terminal until another Take control.
    pub fn arena_keys(&mut self, focused: bool, keys: Keys) {
        if !focused || !self.can_take_control() { self.input.release(); }
        else { self.input.keys(keys); }
        self.pump_input();
    }
    pub(super) fn pump_input(&mut self) {
        if !self.can_take_control() { self.input.release(); }
        if let Some((method, params, intent, op)) = self.input.next(Instant::now()) {
            self.post(method, params, Pend::Input(intent, op));
        }
    }
}
