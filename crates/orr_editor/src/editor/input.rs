//! Explicit, bounded, nonblocking ownership of one realtime input slot.
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
    #[cfg(feature = "static-games")]
    pub registered: Option<crate::static_adapter::RegisteredGame>,
    #[cfg(feature = "mesh-navigation")]
    pub mesh_navigation: bool,
    pending_restarts: u8,
    restart_armed: bool,
    observed_restart: bool,
    #[cfg(feature = "collect-dodge")]
    observed_tick: u64,
    #[cfg(feature = "collect-dodge")]
    collect_observed: bool,
    intent: u64,
    wanted: bool,
    player: u8,
    grant: Option<Grant>,
    latest: Keys,
    sent: Option<Keys>,
    #[cfg(feature = "mesh-navigation")]
    mesh_goal: Option<[i32; 3]>,
    #[cfg(feature = "mesh-navigation")]
    mesh_neutral_pending: bool,
    #[cfg(feature = "mesh-navigation")]
    mesh_flight_goal: Option<[i32; 3]>,
    #[cfg(feature = "mesh-navigation")]
    mesh_waiting_goal: Option<([i32; 3], u64)>,
    #[cfg(feature = "mesh-navigation")]
    mesh_checked_tick: Option<u64>,
    flight: Option<(u64, Op, Instant)>,
    last_ack: Option<Instant>,
}
impl Input {
    pub fn new(enabled: bool) -> Self { Self { enabled, ..Default::default() } }
    pub fn claim(&mut self, player: u8) {
        if self.phase != Phase::Off { return; }
        self.reset_collect_edges();
        self.intent += 1;
        self.player = player;
        self.wanted = true;
        self.phase = Phase::Claiming;
        self.latest = Keys::default();
        self.sent = None;
        #[cfg(feature = "mesh-navigation")]
        { self.mesh_goal = None; self.mesh_neutral_pending = false; self.mesh_flight_goal = None; self.mesh_waiting_goal = None; self.mesh_checked_tick = None; }
    }
    pub fn release(&mut self) {
        self.reset_collect_edges();
        self.wanted = false;
        self.latest = Keys::default();
        #[cfg(feature = "mesh-navigation")]
        { self.mesh_goal = None; self.mesh_neutral_pending = false; self.mesh_flight_goal = None; self.mesh_waiting_goal = None; self.mesh_checked_tick = None; }
        if self.phase != Phase::Off { self.phase = Phase::Releasing; }
    }
    pub fn keys(&mut self, keys: Keys) { if self.wanted {
        #[cfg(feature = "static-games")]
        if self.registered.is_some() && (!(-1..=1).contains(&keys.x) || !(-1..=1).contains(&keys.y)) {
            self.release(); return;
        }
        if self.collect_dodge && self.restart_armed && keys.fire && !self.latest.fire { self.pending_restarts = self.pending_restarts.saturating_add(1).min(8); }
        if !keys.fire { self.restart_armed = true; }
        self.latest = keys;
    } }
    #[cfg(feature = "mesh-navigation")]
    fn mesh_goal(&mut self, goal: [i32; 3]) -> bool {
        if !self.mesh_navigation || !self.wanted || !matches!(self.phase, Phase::Claiming | Phase::Active) { return false; }
        self.mesh_goal = Some(goal);
        self.mesh_neutral_pending = false;
        self.mesh_waiting_goal = None;
        self.mesh_checked_tick = None;
        true
    }
    #[cfg(feature = "mesh-navigation")]
    fn mesh_observe_goal(&mut self, tick: u64, observed_goal: [i32; 3]) {
        if self.mesh_waiting_goal.is_some_and(|(goal, accepted_tick)| tick > accepted_tick && goal == observed_goal) {
            self.mesh_waiting_goal = None;
            self.mesh_checked_tick = None;
            self.mesh_neutral_pending = true;
        }
    }
    fn reset_collect_edges(&mut self) {
        self.restart_armed = false;
        self.pending_restarts = 0;
        self.observed_restart = false;
        #[cfg(feature = "collect-dodge")] { self.observed_tick = 0; self.collect_observed = false; }
    }
    #[cfg(feature = "collect-dodge")]
    fn collect_observe(&mut self, tick: u64, held: bool) {
        if !self.collect_observed || tick < self.observed_tick {
            self.collect_observed = true; self.observed_tick = tick; self.observed_restart = held; return;
        }
        if tick > self.observed_tick {
            if held && !self.observed_restart { self.pending_restarts = self.pending_restarts.saturating_sub(1); }
            self.observed_tick = tick; self.observed_restart = held;
        }
    }
    fn effective_keys(&self) -> Keys {
        let mut keys = self.latest;
        if (self.collect_dodge || self.room_escape) && !self.restart_armed { keys.fire = false; }
        if self.collect_dodge && self.pending_restarts > 0 { keys.fire = !self.observed_restart; }
        keys
    }
    fn off(&mut self) {
        self.reset_collect_edges();
        self.phase = Phase::Off; self.wanted = false; self.grant = None;
        self.flight = None; self.sent = None; self.last_ack = None;
        #[cfg(feature = "mesh-navigation")]
        { self.mesh_goal = None; self.mesh_neutral_pending = false; self.mesh_flight_goal = None; self.mesh_waiting_goal = None; self.mesh_checked_tick = None; }
    }
    pub fn next(&mut self, now: Instant) -> Option<(&'static str, J, u64, Op)> {
        if let Some((_, op, at)) = self.flight {
            if now.duration_since(at) >= Duration::from_millis(2000) {
                self.wanted = false;
                self.latest = Keys::default();
                #[cfg(feature = "mesh-navigation")]
                { self.mesh_goal = None; self.mesh_neutral_pending = false; self.mesh_flight_goal = None; self.mesh_waiting_goal = None; self.mesh_checked_tick = None; }
                if matches!(op, Op::Claim | Op::Release) {
                    self.phase = Phase::Off;
                    return None;
                }
                self.phase = Phase::Releasing;
            }
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
                #[cfg(feature = "mesh-navigation")]
                if self.sent != Some(self.effective_keys()) || (self.mesh_navigation && (self.mesh_goal.is_some() || self.mesh_neutral_pending)) { Op::Value }
                else if self.last_ack.is_some_and(|at| now.duration_since(at) >= Duration::from_millis(500)) { Op::Renew }
                else { return None; }
                #[cfg(not(feature = "mesh-navigation"))]
                if self.sent != Some(self.effective_keys()) { Op::Value }
                else if self.last_ack.is_some_and(|at| now.duration_since(at) >= Duration::from_millis(500)) { Op::Renew }
                else { return None; }
            }
        };
        let (method, params) = if op == Op::Claim {
            ("sim.input_claim", json!({"player":self.player,"replace_held":true}))
        } else {
            let (player, grant_id, generation, sequence) = {
                let grant = self.grant.as_mut()?;
                grant.sequence = grant.sequence.checked_add(1)?;
                (grant.player, grant.grant.clone(), grant.generation.clone(), grant.sequence)
            };
            let mut params = json!({"player":player,"grant":grant_id,"generation":generation,"sequence":sequence.to_string()});
            let method = match op {
                Op::Value => {
                    #[cfg(feature = "mesh-navigation")]
                    if self.mesh_navigation {
                        let value = if let Some(goal) = self.mesh_goal.take() {
                            self.mesh_flight_goal = Some(goal);
                            self.mesh_waiting_goal = None;
                            self.mesh_checked_tick = None;
                            self.mesh_neutral_pending = false;
                            json!({"buttons":orr_remote::mesh_navigation::MESH_SET_GOAL,"origin":goal,"dir":[0,0,0]})
                        } else {
                            self.mesh_neutral_pending = false;
                            json!({"buttons":0,"origin":[0,0,0],"dir":[0,0,0]})
                        };
                        params["value"] = value;
                        self.sent = Some(Keys::default());
                        "sim.input_value"
                    } else {
                        params["value"] = self.legacy_value();
                        "sim.input_value"
                    }
                    #[cfg(not(feature = "mesh-navigation"))]
                    { params["value"] = self.legacy_value(); "sim.input_value" }
                }
                Op::Renew => "sim.input_renew",
                Op::Release => "sim.input_release",
                Op::Claim => unreachable!(),
            };
            (method, params)
        };
        self.flight = Some((self.intent, op, now));
        Some((method, params, self.intent, op))
    }
    fn legacy_value(&mut self) -> J {
        #[cfg(feature = "static-games")]
        if let Some(game) = self.registered {
            let keys = self.effective_keys();
            // keys() admits only finite axes, including the neutral claim state.
            let value = game.held_value(crate::static_adapter::StaticHeldKeys { x: keys.x, y: keys.y, action: keys.fire })
                .expect("registered input keys were admitted");
            self.sent = Some(keys);
            return value;
        }
        let value = if self.room_escape { json!({"move_x":self.latest.x,"move_z":-self.latest.y,"buttons":if self.effective_keys().fire {vec!["interact"]}else{vec![]}}) }
        else if self.collect_dodge { json!({"x":self.latest.x,"y":self.latest.y,"buttons":if self.effective_keys().fire {vec!["restart"]}else{vec![]}}) }
        else { self.effective_keys().value() };
        self.sent = Some(self.effective_keys());
        value
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
        #[cfg(feature = "mesh-navigation")]
        if op == Op::Value {
            if let Some(goal) = self.mesh_flight_goal.take() {
                self.mesh_waiting_goal = value["accepted_head_tick"].as_u64().map(|tick| (goal, tick));
                self.mesh_checked_tick = None;
            }
        }
        self.last_ack = Some(sent_at);
        None
    }
}

impl Editor {
    pub fn input_phase(&self) -> Phase { self.input.phase }
    pub fn input_hint(&self) -> &'static str {
        if self.input_cleanup_pending() { "Off · cleanup reply pending; lease expiry is the fallback" }
        else if !self.input.enabled { "Off · compatible managed input unavailable" }
        else if self.is_viewer() { "Off · replay Viewer is read-only" }
        else if self.preview.is_some() { "Off · proposal preview" }
        else if !self.sim.playing { "Off · start realtime Play to take control" }
        else if self.sim.head_tick != self.sim.last_tick { "Off · wait for live head" }
        else if !self.mesh_gameplay_input_open() { "Off · gameplay ended or its authoritative state is unavailable" }
        else { "Off · explicit Take control required" }
    }
    pub fn input_cleanup_pending(&self) -> bool {
        self.input.phase == Phase::Off && self.pending.values().any(|p| matches!(p.kind, Pend::Input(..)))
    }
    pub fn can_take_control(&self) -> bool {
        (self.game().has_keyboard() || self.game().has_mesh_view()) && !self.input_cleanup_pending() && self.input.enabled && self.down.is_none()
            && !self.is_viewer() && self.preview.is_none() && self.sim.mode == Mode::Play
            && self.sim.playing && self.sim.head_tick == self.sim.last_tick
            && self.mesh_gameplay_input_open()
    }
    /// Terminal or invalid hazard readback cannot acquire or retain managed input.
    pub(crate) fn mesh_gameplay_input_open(&self) -> bool {
        #[cfg(feature = "mesh-hazards")]
        if self.game().is_mesh_hazards() {
            let Some(snapshot) = self.snapshot() else { return false; };
            let frame = snapshot.predicted();
            let hazards = orr_remote::mesh_navigation::hazards::from_view(frame);
            let mission = orr_remote::mesh_navigation::mission::from_view(frame);
            return hazards.is_ok_and(|view| view.is_none_or(|view| view.run.lost == 0))
                && mission.is_ok_and(|view| view.is_none_or(|view| view.run.won == 0));
        }
        true
    }

    pub fn take_control(&mut self, player: u8, viewport_focused: bool) -> bool {
        #[cfg(feature = "static-games")]
        if self.game().is_registered() && player != 0 { return false; }
        if !viewport_focused || self.game().has_mesh_view() || !self.can_take_control() || player >= self.sim.player_count || self.input.phase != Phase::Off || self.pending.values().any(|p| matches!(p.kind,Pend::Input(..))) { return false; }
        self.state_generation += 1;
        self.input.claim(player); self.pump_input(); true
    }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn mesh_play_input_enabled(&self) -> bool { self.input.mesh_navigation && self.input.wanted }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn mesh_play_input_phase(&self) -> Phase { self.input.phase }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn mesh_play_input_hint(&self) -> &'static str {
        if !self.game().has_mesh_view() { "Unavailable · open a mesh navigation game" }
        else if !self.input.enabled { "Unavailable · compatible managed input is not advertised by this host" }
        else if self.input.phase == Phase::Claiming { "Claiming managed player input…" }
        else if self.input.phase == Phase::Active { "Managed player input active" }
        else if self.input.phase == Phase::Releasing { "Releasing managed player input…" }
        else { self.input_hint() }
    }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn set_mesh_play_input_enabled(&mut self, enabled: bool) -> bool {
        if !enabled { if self.input.mesh_navigation { self.release_control(); } return true; }
        if !self.game().has_mesh_view() || !self.can_take_control() || self.input.phase != Phase::Off
            || self.pending.values().any(|pending| matches!(pending.kind,Pend::Input(..))) || self.sim.player_count == 0 { return false; }
        self.input.mesh_navigation = true;
        self.state_generation += 1;
        self.input.claim(0);
        self.pump_input();
        true
    }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn queue_mesh_play_goal(&mut self, point: [FP; 3]) -> Result<(), String> {
        if !self.game().has_mesh_view() || !self.input.mesh_navigation || !self.input.wanted
            || !matches!(self.input.phase, Phase::Claiming | Phase::Active) { return Err("Enable managed mesh goal input before selecting a goal".into()); }
        if !self.can_take_control() { self.release_control(); return Err("Mesh goal input requires the current live Play head".into()); }
        let bound = 256_i64 * FP::SCALE;
        let mut raw = [0_i32; 3];
        for (raw_coordinate, coordinate) in raw.iter_mut().zip(point) {
            let value = coordinate.raw();
            if value < -bound || value > bound { return Err("Mesh goal coordinates must be within ±256 world units".into()); }
            *raw_coordinate = i32::try_from(value).map_err(|_| "Mesh goal coordinate exceeds the YardInput range".to_string())?;
        }
        if !self.input.mesh_goal(raw) { return Err("Managed mesh input is no longer active".into()); }
        self.pump_input();
        Ok(())
    }
    pub fn collect_space_event(&mut self, focused: bool, down: bool) {
        if !self.game().is_collect() || !focused || !self.can_take_control() { return; }
        if self.input.wanted {
            self.input.restart_armed = true;
            self.input.keys(Keys { fire:down, ..self.input.latest });
            self.pump_input();
        }
    }
    pub fn release_control(&mut self) { self.input.release(); self.pump_input(); }
    pub fn arena_keys(&mut self, focused: bool, keys: Keys) {
        if !focused || !self.can_take_control() { self.input.release(); }
        else { self.input.keys(keys); }
        self.pump_input();
    }
    pub(super) fn pump_input(&mut self) {
        #[cfg(feature = "collect-dodge")]
        if self.game().is_collect() {
            if let Some(snapshot) = self.snapshot.as_ref() {
                self.input.collect_observe(snapshot.tick(), snapshot.predicted().singleton::<orr_sample::collect_game::CollectRun>().restart_held != 0);
            }
        }
        #[cfg(feature = "mesh-navigation")]
        if self.game().has_mesh_view() {
            if let (Some(snapshot), Some((_, accepted_tick))) = (self.snapshot.as_ref(), self.input.mesh_waiting_goal) {
                let tick = snapshot.tick();
                if tick > accepted_tick && self.input.mesh_checked_tick != Some(tick) {
                    self.input.mesh_checked_tick = Some(tick);
                    if let Ok(view) = orr_remote::mesh_navigation::mesh_from_view(snapshot.predicted()) {
                        let raw_goal = view.agent.spec.goal.map(FP::raw);
                        if let (Ok(coordinate_x), Ok(coordinate_y), Ok(coordinate_z)) = (
                            i32::try_from(raw_goal[0]),
                            i32::try_from(raw_goal[1]),
                            i32::try_from(raw_goal[2]),
                        ) {
                            self.input.mesh_observe_goal(tick, [coordinate_x, coordinate_y, coordinate_z]);
                        }
                    }
                }
            }
        }
        if !self.can_take_control() { self.input.release(); }
        if let Some((method,params,intent,op)) = self.input.next(Instant::now()) { self.post(method,params,Pend::Input(intent,op)); }
    }
}

#[cfg(all(test, feature = "mesh-navigation"))]
mod mesh_play_tests {
    use super::*;

    fn claim_ack() -> J { json!({"player":0,"grant":"9","generation":"3","lease_ms":2000,"accepted_head_tick":5}) }

    #[test]
    fn mesh_goal_during_claim_is_coalesced_then_sent_once_and_neutralized() {
        let mut input = Input::new(true);
        input.mesh_navigation = true;
        input.claim(0);
        assert!(input.mesh_goal([1,2,3]));
        assert!(input.mesh_goal([-4,5,-6]));
        let now = Instant::now();
        let (_,_,intent,op) = input.next(now).unwrap();
        assert_eq!(op, Op::Claim);
        input.answer(intent,op,Ok(claim_ack()),now);
        let (_,goal,intent,op) = input.next(now).unwrap();
        assert_eq!(op, Op::Value);
        assert_eq!(goal["value"],json!({"buttons":orr_remote::mesh_navigation::MESH_SET_GOAL,"origin":[-4,5,-6],"dir":[0,0,0]}));
        let mut ack = goal;
        ack["ok"] = json!(true); ack["accepted_head_tick"] = json!(6);
        ack["player"] = json!(0); ack["grant"] = json!("9"); ack["generation"] = json!("3"); ack["sequence"] = json!("1");
        input.answer(intent,op,Ok(ack),now);
        assert!(input.next(now).is_none(), "the goal remains held until a later frame confirms consumption");
        input.mesh_observe_goal(6, [-4,5,-6]);
        assert!(input.next(now).is_none(), "the accepted head tick itself is not consumption evidence");
        input.mesh_observe_goal(7, [-4,5,-6]);
        let (_,neutral,_,op) = input.next(now).unwrap();
        assert_eq!(op,Op::Value);
        assert_eq!(neutral["value"],json!({"buttons":0,"origin":[0,0,0],"dir":[0,0,0]}));
    }

    #[test]
    fn mesh_release_and_timeout_discard_queued_goals() {
        let now = Instant::now();
        let mut input = Input::new(true); input.mesh_navigation = true; input.claim(0);
        assert!(input.mesh_goal([1,2,3]));
        let (_,_,intent,op) = input.next(now).unwrap(); assert_eq!(op,Op::Claim);
        input.release();
        assert!(input.next(now).is_none());
        input.answer(intent,op,Ok(claim_ack()),now);
        let (_,_,_,op) = input.next(now).unwrap(); assert_eq!(op,Op::Release);
        assert!(!input.mesh_goal([4,5,6]));

        let mut input = Input::new(true); input.mesh_navigation = true; input.claim(0);
        assert!(input.mesh_goal([1,2,3])); input.next(now).unwrap();
        assert!(input.next(now + Duration::from_secs(2)).is_none());
        assert!(!input.mesh_goal([4,5,6]));
    }
}

#[cfg(all(test, feature = "mesh-navigation"))]
#[path = "mesh_play_input_managed_tests.rs"]
mod mesh_play_input_managed_tests;

#[cfg(all(test, feature = "collect-dodge"))]
mod collect_tests {
    use super::*;
    fn input() -> Input { let mut input=Input::new(true);input.collect_dodge=true;input.claim(0);input.collect_observe(100,false);input.keys(Keys::default());input }
    fn tap(input:&mut Input){input.keys(Keys{fire:true,..Default::default()});input.keys(Keys::default());}
    #[test]
    fn collect_queued_edges_require_observed_ticks_and_neutral_rearm(){
        let mut input=input();tap(&mut input);tap(&mut input);
        assert!(input.effective_keys().fire);assert_eq!(input.pending_restarts,2);
        input.collect_observe(100,true);assert_eq!(input.pending_restarts,2);
        input.collect_observe(101,true);assert_eq!(input.pending_restarts,1);assert!(!input.effective_keys().fire);
        input.collect_observe(102,false);assert!(input.effective_keys().fire);
        input.collect_observe(103,true);assert_eq!(input.pending_restarts,0);assert!(!input.effective_keys().fire);
    }
    #[test]
    fn collect_stop_error_and_reclaim_cancel_edges_and_prime_new_epoch(){
        for terminal in [false,true] {
            let mut input=input();tap(&mut input);
            if terminal {input.off();} else {input.release();input.off();}
            assert_eq!(input.pending_restarts,0);
            input.claim(0);input.collect_observe(1,true);input.keys(Keys::default());assert_eq!(input.pending_restarts,0);
            tap(&mut input);assert!(!input.effective_keys().fire);
            input.collect_observe(2,false);assert!(input.effective_keys().fire);
            input.collect_observe(3,true);assert_eq!(input.pending_restarts,0);
        }
    }
    #[test]
    fn collect_reclaim_does_not_turn_old_held_space_into_a_new_press() {
        let mut input=Input::new(true);input.collect_dodge=true;input.claim(0);input.collect_observe(5,false);
        input.keys(Keys{fire:true,..Default::default()});assert_eq!(input.pending_restarts,0);assert!(!input.effective_keys().fire);
        input.keys(Keys::default());input.keys(Keys{fire:true,..Default::default()});assert_eq!(input.pending_restarts,1);assert!(input.effective_keys().fire);
        input.off();assert_eq!(input.pending_restarts,0);input.claim(0);input.collect_observe(6,false);
        input.keys(Keys{fire:true,..Default::default()});assert_eq!(input.pending_restarts,0);assert!(!input.effective_keys().fire);
    }
}

#[cfg(all(test,feature="room-project"))]
mod room_input_tests {
    use super::*;
    #[test]
    fn room_claim_and_reclaim_require_neutral_interact_before_fresh_press() {
        let mut input=Input::new(true);input.room_escape=true;
        let held=Keys{fire:true,..Keys::default()};
        input.claim(0);input.keys(held);
        assert!(!input.effective_keys().fire,"claim must not turn old held E into a press");
        input.keys(Keys::default());input.keys(held);
        assert!(input.effective_keys().fire);
        input.release();input.off();input.claim(0);input.keys(held);
        assert!(!input.effective_keys().fire,"reclaim must require a release again");
        input.keys(Keys::default());input.keys(held);
        assert!(input.effective_keys().fire);
    }
}

#[cfg(all(test, feature = "mesh-hazards"))]
mod mesh_hazards_control_tests {
    use super::*;
    use crate::game::EditorGame;
    use orr_remote::mesh_navigation::{hazards, mesh_from_view};

    fn fixture() -> (tempfile::TempDir, Editor) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hazards.scene.yaml");
        drop(crate::mesh_workspace::mission::prepare(&path, &Default::default()).unwrap());
        let mut editor = Editor::open_game(&path, EditorGame::MeshHazardsV1).unwrap();
        editor.sync();
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        let disabled = orr_remote::mesh_navigation::mission::MeshMission::default();
        editor.host_call("world.singleton.patch", orr_remote::mesh_navigation::mission::patch_request(&disabled, &view).unwrap()).unwrap();
        editor.sync();
        (directory, editor)
    }

    fn wait(editor: &mut Editor, predicate: impl Fn(&Editor) -> bool) {
        wait_for(editor, "managed input state", predicate);
    }

    fn wait_for(editor: &mut Editor, stage: &str, predicate: impl Fn(&Editor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            editor.sync();
            if predicate(editor) { break; }
            assert!(Instant::now() < deadline, "managed hazard input state timed out: stage={stage} tick/lost={:?} phase={:?} can={} hint={} log={:?}", editor.snapshot().map(|snapshot| { let frame = snapshot.predicted(); (frame.tick(), hazards::from_view(frame).map(|view| view.map(|view| view.run.lost))) }), editor.input_phase(), editor.can_take_control(), editor.input_hint(), editor.log().iter().map(|m| &m.text).collect::<Vec<_>>());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn hazards_acquire_managed_control_queue_goal_and_stop_rejects_input() {
        let (_directory, mut editor) = fixture();
        editor.play();
        wait(&mut editor, Editor::can_take_control);
        assert!(!editor.take_control(0, true), "mesh profile must use managed goal input");
        assert!(editor.set_mesh_play_input_enabled(true));
        wait(&mut editor, |editor| editor.input_phase() == Phase::Active);
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        assert!(editor.queue_mesh_play_goal(view.agent.spec.start).is_ok());
        editor.stop();
        editor.sync();
        assert!(!editor.can_take_control());
        assert!(!editor.set_mesh_play_input_enabled(true));
        assert!(editor.queue_mesh_play_goal(view.agent.spec.goal).is_err());
    }

    #[test]
    fn terminal_hazard_loss_never_acquires_or_queues_input() {
        let (_directory, mut editor) = fixture();
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        let mut config = hazards::MeshHazards {
            enabled: 1, count: 1, graph_revision: view.graph.revision(), ..Default::default()
        };
        config.spheres[0] = hazards::HazardSphere { center: view.agent.position, radius: FP::from_int(1) };
        editor.host_call("world.singleton.patch", hazards::patch_request(&config, &view).unwrap()).unwrap();
        editor.sync();
        editor.play();
        wait(&mut editor, |editor| hazards::from_view(editor.snapshot().unwrap().predicted()).unwrap().is_some_and(|view| view.run.lost == 1));
        assert!(!editor.mesh_gameplay_input_open());
        assert!(!editor.can_take_control());
        assert!(!editor.set_mesh_play_input_enabled(true));
        assert!(editor.queue_mesh_play_goal(view.agent.spec.goal).is_err());
        editor.stop();
        editor.sync();
        assert!(editor.mesh_gameplay_input_open());
    }
    #[test]
    fn terminal_mission_win_never_acquires_or_queues_input() {
        let (_directory, mut editor) = fixture();
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        use orr_remote::mesh_navigation::mission;
        let point = mission::MissionPoint { position: view.agent.position, radius: FP::from_int(1) };
        let mut config = mission::MeshMission {
            enabled: 1, count: 1, exit: point, graph_revision: view.graph.revision(), ..Default::default()
        };
        config.points[0] = point;
        editor.host_call("world.singleton.patch", mission::patch_request(&config, &view).unwrap()).unwrap();
        editor.sync();
        editor.play();
        wait(&mut editor, |editor| mission::from_view(editor.snapshot().unwrap().predicted()).unwrap().is_some_and(|view| view.run.won == 1));
        assert!(!editor.mesh_gameplay_input_open());
        assert!(!editor.can_take_control());
        assert!(!editor.set_mesh_play_input_enabled(true));
        assert!(editor.queue_mesh_play_goal(view.agent.spec.goal).is_err());
    }

    #[test]
    fn hazard_loss_releases_an_active_managed_grant() {
        let (_directory, mut editor) = fixture();
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        let original_goal = view.agent.spec.goal;
        // Admit a stationary initial route so host scheduling cannot reach the
        // hazard before the managed grant is observed. Motion starts only below.
        let mut pin = view.pin;
        pin.goal = pin.start;
        let types = hazards::hazards_types();
        let value = types.get(orr_remote::mesh_navigation::PIN_NAME).unwrap()
            .read(orr_reflect::bytemuck::bytes_of(&pin));
        editor.host_call("world.singleton.patch", serde_json::json!({
            "name": orr_remote::mesh_navigation::PIN_NAME,
            "value": orr_remote::json::value_to_json(&value),
        })).unwrap();
        editor.sync();
        let stationary = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        assert_eq!(stationary.pin.goal, stationary.pin.start);
        assert_eq!(stationary.agent.position, view.agent.position);
        let mut config = hazards::MeshHazards {
            enabled: 1, count: 1, graph_revision: view.graph.revision(), ..Default::default()
        };
        config.spheres[0] = hazards::HazardSphere { center: original_goal, radius: FP::from_raw(FP::SCALE / 4) };
        editor.host_call("world.singleton.patch", hazards::patch_request(&config, &stationary).unwrap()).unwrap();
        editor.sync();
        editor.play();
        wait_for(&mut editor, "stationary Play ready", Editor::can_take_control);
        assert!(editor.set_mesh_play_input_enabled(true));
        wait_for(&mut editor, "stationary managed grant Active", |editor| editor.input_phase() == Phase::Active);
        assert!(editor.mesh_gameplay_input_open());
        assert_eq!(mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap().agent.position, stationary.agent.position);
        assert!(editor.queue_mesh_play_goal(original_goal).is_ok());
        wait_for(&mut editor, "queued goal reaches hazard", |editor| hazards::from_view(editor.snapshot().unwrap().predicted()).unwrap().is_some_and(|view| view.run.lost == 1));
        wait_for(&mut editor, "terminal managed grant released", |editor| editor.input_phase() == Phase::Off && !editor.input_cleanup_pending());
        assert!(!editor.mesh_play_input_enabled());
        assert!(editor.queue_mesh_play_goal(view.agent.spec.start).is_err());
    }

}

#[cfg(all(test, feature = "static-games"))]
mod registered_input_tests {
    use super::*;
    fn input() -> Input {
        let mut input = Input::new(true);
        input.registered = Some(crate::static_adapter::tests::registry().handle("FixtureGame").unwrap());
        input
    }
    fn claim_ack() -> J { json!({"player":0,"grant":"9","generation":"3","lease_ms":2000,"accepted_head_tick":0}) }
    #[test]
    fn registered_mapping_only_supplies_value_editor_owns_envelope_and_release() {
        let mut input=input(); let now=Instant::now(); input.claim(0);
        let (method,params,intent,op)=input.next(now).unwrap();
        assert_eq!(method,"sim.input_claim");assert_eq!(params["player"],0);
        input.answer(intent,op,Ok(claim_ack()),now);
        input.keys(Keys{x:1,y:-1,fire:true});
        let (method,params,_,op)=input.next(now).unwrap();
        assert_eq!(method,"sim.input_value");assert_eq!(op,Op::Value);
        assert_eq!(params["value"],json!({"axis_x":1,"axis_y":-1,"buttons":["fire"]}));
        assert_eq!(params["player"],0);assert_eq!(params["grant"],"9");assert_eq!(params["generation"],"3");assert_eq!(params["sequence"],"1");
        input.release();
        let (method,_,_,op)=input.next(now).unwrap();assert_eq!(method,"sim.input_release");assert_eq!(op,Op::Release);
    }
    #[test]
    fn registered_delayed_claim_release_and_disconnect_never_reclaim() {
        let mut input=input();let now=Instant::now();input.claim(0);
        let (_,_,intent,op)=input.next(now).unwrap();input.release();
        input.answer(intent,op,Ok(claim_ack()),now);
        assert_eq!(input.phase,Phase::Releasing);
        assert_eq!(input.next(now).unwrap().0,"sim.input_release");
        input.off();assert!(input.next(now).is_none());assert!(!input.wanted);
        input.claim(0);assert_eq!(input.latest,Keys::default());
        let (_,_,intent,op)=input.next(now).unwrap();
        input.answer(intent,op,Ok(claim_ack()),now+Duration::from_millis(2001));
        assert_eq!(input.phase,Phase::Releasing);assert!(!input.wanted);
    }
    #[test]
    fn registered_invalid_axes_release_instead_of_clamp_or_fallback() {
        let mut input=input();let now=Instant::now();input.claim(0);
        let (_,_,intent,op)=input.next(now).unwrap();input.answer(intent,op,Ok(claim_ack()),now);
        input.keys(Keys{x:2,y:0,fire:false});assert_eq!(input.phase,Phase::Releasing);
        assert_eq!(input.next(now).unwrap().0,"sim.input_release");
    }
}
