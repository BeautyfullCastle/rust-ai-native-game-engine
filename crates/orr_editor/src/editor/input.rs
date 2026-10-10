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
    pub collect_dodge: bool,
    pub room_escape: bool,
    pending_presses: u8,
    restart_armed: bool,
    observed_restart: bool,
    #[cfg(any(feature = "collect-dodge", feature = "room-project"))]
    observed_tick: u64,
    #[cfg(any(feature = "collect-dodge", feature = "room-project"))]
    button_observed: bool,
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
        self.reset_button_edges();
        self.intent += 1;
        self.player = player;
        self.wanted = true;
        self.phase = Phase::Claiming;
        self.latest = Keys::default();
        self.sent = None;
    }
    pub fn release(&mut self) {
        self.reset_button_edges();
        self.wanted = false;
        self.latest = Keys::default();
        if self.phase != Phase::Off { self.phase = Phase::Releasing; }
    }
    pub fn keys(&mut self, keys: Keys) { if self.wanted {
        if (self.collect_dodge || self.room_escape) && self.restart_armed && keys.fire && !self.latest.fire { self.pending_presses = self.pending_presses.saturating_add(1).min(8); }
        if !keys.fire { self.restart_armed = true; }
        self.latest = keys;
    } }
    fn reset_button_edges(&mut self) {
        self.restart_armed = false;
        self.pending_presses = 0;
        self.observed_restart = false;
        #[cfg(any(feature = "collect-dodge", feature = "room-project"))] { self.observed_tick = 0; self.button_observed = false; }
    }
    #[cfg(any(feature = "collect-dodge", feature = "room-project"))]
    fn button_observe(&mut self, tick: u64, held: bool) {
        if !self.button_observed || tick < self.observed_tick {
            self.button_observed = true; self.observed_tick = tick; self.observed_restart = held; return;
        }
        if tick > self.observed_tick {
            if held && !self.observed_restart { self.pending_presses = self.pending_presses.saturating_sub(1); }
            self.observed_tick = tick; self.observed_restart = held;
        }
    }
    fn effective_keys(&self) -> Keys {
        let mut keys = self.latest;
        if (self.collect_dodge || self.room_escape) && !self.restart_armed { keys.fire = false; }
        if (self.collect_dodge || self.room_escape) && self.pending_presses > 0 { keys.fire = !self.observed_restart; }
        keys
    }
    fn off(&mut self) {
        self.reset_button_edges();
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
                if self.sent != Some(self.effective_keys()) { Op::Value }
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
                Op::Value => { params["value"] = if self.room_escape { json!({"move_x":self.latest.x,"move_z":-self.latest.y,"buttons":if self.effective_keys().fire {vec!["interact"]}else{vec![]}}) } else if self.collect_dodge { json!({"x":self.latest.x,"y":self.latest.y,"buttons":if self.effective_keys().fire {vec!["restart"]}else{vec![]}}) } else { self.latest.value() }; self.sent = Some(self.effective_keys()); "sim.input_value" },
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
        self.game().has_keyboard() && !self.input_cleanup_pending() && self.input.enabled && self.down.is_none()
            && !self.is_viewer() && self.preview.is_none() && self.sim.mode == Mode::Play
            && self.sim.playing && self.sim.head_tick == self.sim.last_tick
    }
    /// Explicit human intent only; the caller must deliberately focus the viewport.
    pub fn take_control(&mut self, player: u8, viewport_focused: bool) -> bool {
        if !viewport_focused || !self.can_take_control() || player >= self.sim.player_count || self.input.phase != Phase::Off || self.pending.values().any(|p| matches!(p.kind, Pend::Input(..))) { return false; }
        self.state_generation += 1;
        self.input.claim(player); self.pump_input(); true
    }
    /// A real ordered Space transition, distinct from an already-held sample on reclaim.
    pub fn collect_space_event(&mut self, focused: bool, down: bool) {
        if !self.game().is_collect() || !focused || !self.can_take_control() { return; }
        if self.input.wanted {
            self.input.restart_armed = true;
            self.input.keys(Keys { fire:down, ..self.input.latest });
            self.pump_input();
        }
    }
    /// Ordered non-repeat E transitions preserve short taps between UI frames.
    /// Focus alone never authorizes control; held E on reclaim stays neutral.
    pub fn room_interact_event(&mut self, focused: bool, down: bool) {
        if !self.game().is_room() || !focused || !self.can_take_control() { return; }
        if self.input.wanted {
            self.input.restart_armed = true;
            self.input.keys(Keys { fire:down, ..self.input.latest });
            self.pump_input();
        }
    }
    pub fn release_control(&mut self) { self.input.release(); self.pump_input(); }
    /// UI focus/capture gate. Losing focus is terminal until another Take control.
    pub fn arena_keys(&mut self, focused: bool, keys: Keys) {
        if !focused || !self.can_take_control() { self.input.release(); }
        else { self.input.keys(keys); }
        self.pump_input();
    }
    pub(super) fn pump_input(&mut self) {
        #[cfg(feature = "collect-dodge")]
        if self.game().is_collect() {
            if let Some(snapshot) = self.snapshot.as_ref() {
                self.input.button_observe(snapshot.tick(), snapshot.predicted().singleton::<orr_sample::collect_game::CollectRun>().restart_held != 0);
            }
        }
        #[cfg(feature = "room-project")]
        if self.game().is_room() {
            if let Some(snapshot) = self.snapshot.as_ref() {
                let run = snapshot.predicted().singleton::<orr_sample::room_game::RoomRun>();
                if run.won != 0 {
                    // Completed rooms stop updating previous_buttons. Do not retain
                    // queued intent across a terminal frame or a later restart.
                    self.input.reset_button_edges();
                } else {
                    self.input.button_observe(snapshot.tick(), run.previous_buttons & orr_sample::room_game::INTERACT != 0);
                }
            }
        }
        if !self.can_take_control() { self.input.release(); }
        if let Some((method, params, intent, op)) = self.input.next(Instant::now()) {
            self.post(method, params, Pend::Input(intent, op));
        }
    }
}

#[cfg(all(test, feature = "collect-dodge"))]
mod collect_tests {
    use super::*;
    fn input() -> Input { let mut i=Input::new(true); i.collect_dodge=true;i.claim(0);i.button_observe(100,false);i.keys(Keys::default());i }
    fn tap(i:&mut Input){ i.keys(Keys{fire:true,..Default::default()});i.keys(Keys::default()); }
    #[test]
    fn collect_queued_edges_require_observed_ticks_and_neutral_rearm(){
        let mut i=input();tap(&mut i);tap(&mut i);
        assert!(i.effective_keys().fire);assert_eq!(i.pending_presses,2);
        i.button_observe(100,true);assert_eq!(i.pending_presses,2);
        i.button_observe(101,true);assert_eq!(i.pending_presses,1);assert!(!i.effective_keys().fire);
        i.button_observe(102,false);assert!(i.effective_keys().fire);
        i.button_observe(103,true);assert_eq!(i.pending_presses,0);assert!(!i.effective_keys().fire);
    }
    #[test]
    fn collect_stop_error_and_reclaim_cancel_edges_and_prime_new_epoch(){
        for terminal in [false,true] {
            let mut i=input();tap(&mut i);
            if terminal {i.off();} else {i.release();i.off();}
            assert_eq!(i.pending_presses,0);
            i.claim(0);i.button_observe(1,true);i.keys(Keys::default());assert_eq!(i.pending_presses,0);
            tap(&mut i);assert!(!i.effective_keys().fire);
            i.button_observe(2,false);assert!(i.effective_keys().fire);
            i.button_observe(3,true);assert_eq!(i.pending_presses,0);
        }
    }
    #[test]
    fn collect_reclaim_does_not_turn_old_held_space_into_a_new_press() {
        let mut i=Input::new(true);i.collect_dodge=true;i.claim(0);i.button_observe(5,false);
        i.keys(Keys{fire:true,..Default::default()});assert_eq!(i.pending_presses,0);assert!(!i.effective_keys().fire);
        i.keys(Keys::default());i.keys(Keys{fire:true,..Default::default()});assert_eq!(i.pending_presses,1);assert!(i.effective_keys().fire);
        i.off();assert_eq!(i.pending_presses,0);i.claim(0);i.button_observe(6,false);
        i.keys(Keys{fire:true,..Default::default()});assert_eq!(i.pending_presses,0);assert!(!i.effective_keys().fire);
    }

}

#[cfg(all(test,feature="room-project"))]
mod room_input_tests {
    use super::*;
    fn room_input() -> Input {
        let mut input = Input::new(true);
        input.room_escape = true;
        input.claim(0);
        input.button_observe(100, false);
        input.keys(Keys::default());
        input
    }
    fn tap(input: &mut Input) {
        input.keys(Keys { fire: true, ..Keys::default() });
        input.keys(Keys::default());
    }
    #[test]
    fn room_two_short_taps_wait_for_distinct_observed_press_and_release_ticks() {
        let mut input = room_input();
        tap(&mut input);
        tap(&mut input);
        assert_eq!(input.pending_presses, 2);
        assert!(input.effective_keys().fire);
        input.button_observe(100, true);
        assert_eq!(input.pending_presses, 2, "same tick is not consumption");
        input.button_observe(101, true);
        assert_eq!(input.pending_presses, 1);
        assert!(!input.effective_keys().fire, "release must reach the simulation");
        input.button_observe(105, true);
        assert_eq!(input.pending_presses, 1, "held frames cannot consume another tap");
        assert!(!input.effective_keys().fire);
        input.button_observe(106, false);
        assert!(input.effective_keys().fire);
        input.button_observe(109, true);
        assert_eq!(input.pending_presses, 0);
        assert!(!input.effective_keys().fire);
    }
    #[test]
    fn room_held_interact_is_one_press_and_queue_is_bounded() {
        let mut input = room_input();
        for _ in 0..100 { input.keys(Keys { fire: true, ..Keys::default() }); }
        assert_eq!(input.pending_presses, 1);
        input.button_observe(101, true);
        assert_eq!(input.pending_presses, 0);
        assert!(input.effective_keys().fire, "held state stays held without synthetic retrigger");
        input.keys(Keys::default());
        for _ in 0..100 { tap(&mut input); }
        assert_eq!(input.pending_presses, 8);
        input.release();
        assert_eq!(input.pending_presses, 0);
        assert!(!input.effective_keys().fire);
    }
    #[test]
    fn room_terminal_release_and_reclaim_discard_queued_taps() {
        for off in [false, true] {
            let mut input = room_input();
            tap(&mut input);
            tap(&mut input);
            if off { input.off(); } else { input.release(); input.off(); }
            assert_eq!(input.pending_presses, 0);
            input.claim(0);
            input.button_observe(1, true);
            input.keys(Keys { fire: true, ..Keys::default() });
            assert_eq!(input.pending_presses, 0);
            assert!(!input.effective_keys().fire);
            input.keys(Keys::default());
            tap(&mut input);
            assert!(!input.effective_keys().fire, "old held simulation state needs neutral tick");
            input.button_observe(2, false);
            assert!(input.effective_keys().fire);
        }
    }

    #[test]
    fn room_tap_before_claim_ack_survives_until_observed_and_late_claim_cancels() {
        for cancelled in [false, true] {
            let now = Instant::now();
            let mut input = room_input();
            let (_, _, intent, op) = input.next(now).unwrap();
            tap(&mut input);
            if cancelled { input.release(); }
            let ack = json!({"player":0,"grant":"9","generation":"3","lease_ms":2000,"accepted_head_tick":100});
            assert!(input.answer(intent, op, Ok(ack), now).is_none());
            let (method, params, _, _) = input.next(now).unwrap();
            if cancelled {
                assert_eq!(method, "sim.input_release");
                assert_eq!(input.pending_presses, 0);
            } else {
                assert_eq!(method, "sim.input_value");
                assert_eq!(params["value"]["buttons"], json!(["interact"]));
                assert_eq!(input.pending_presses, 1, "admission acknowledgment is not simulation consumption");
                input.button_observe(101, true);
                assert_eq!(input.pending_presses, 0);
                assert!(!input.effective_keys().fire);
            }
        }
    }
    #[test]
    fn room_claim_and_reclaim_require_neutral_interact_before_fresh_press() {
        let mut input=Input::new(true); input.room_escape=true;
        let held=Keys { fire:true,..Keys::default() };
        input.claim(0); input.keys(held);
        assert!(!input.effective_keys().fire,"claim must not turn old held E into a press");
        input.keys(Keys::default()); input.keys(held);
        assert!(input.effective_keys().fire);
        input.release(); input.off(); input.claim(0); input.keys(held);
        assert!(!input.effective_keys().fire,"reclaim must require a release again");
        input.keys(Keys::default()); input.keys(held);
        assert!(input.effective_keys().fire);
    }
}
