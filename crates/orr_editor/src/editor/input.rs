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
        else { "Off · explicit Take control required" }
    }
    pub fn input_cleanup_pending(&self) -> bool {
        self.input.phase == Phase::Off && self.pending.values().any(|p| matches!(p.kind, Pend::Input(..)))
    }
    pub fn can_take_control(&self) -> bool {
        (self.game().has_keyboard() || self.game().is_mesh_navigation()) && !self.input_cleanup_pending() && self.input.enabled && self.down.is_none()
            && !self.is_viewer() && self.preview.is_none() && self.sim.mode == Mode::Play
            && self.sim.playing && self.sim.head_tick == self.sim.last_tick
    }
    pub fn take_control(&mut self, player: u8, viewport_focused: bool) -> bool {
        if !viewport_focused || self.game().is_mesh_navigation() || !self.can_take_control() || player >= self.sim.player_count || self.input.phase != Phase::Off || self.pending.values().any(|p| matches!(p.kind,Pend::Input(..))) { return false; }
        self.state_generation += 1;
        self.input.claim(player); self.pump_input(); true
    }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn mesh_play_input_enabled(&self) -> bool { self.input.mesh_navigation && self.input.wanted }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn mesh_play_input_phase(&self) -> Phase { self.input.phase }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn mesh_play_input_hint(&self) -> &'static str {
        if !self.game().is_mesh_navigation() { "Unavailable · open a mesh navigation game" }
        else if !self.input.enabled { "Unavailable · compatible managed input is not advertised by this host" }
        else if self.input.phase == Phase::Claiming { "Claiming managed player input…" }
        else if self.input.phase == Phase::Active { "Managed player input active" }
        else if self.input.phase == Phase::Releasing { "Releasing managed player input…" }
        else { self.input_hint() }
    }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn set_mesh_play_input_enabled(&mut self, enabled: bool) -> bool {
        if !enabled { if self.input.mesh_navigation { self.release_control(); } return true; }
        if !self.game().is_mesh_navigation() || !self.can_take_control() || self.input.phase != Phase::Off
            || self.pending.values().any(|pending| matches!(pending.kind,Pend::Input(..))) || self.sim.player_count == 0 { return false; }
        self.input.mesh_navigation = true;
        self.state_generation += 1;
        self.input.claim(0);
        self.pump_input();
        true
    }
    #[cfg(feature = "mesh-navigation")]
    pub(crate) fn queue_mesh_play_goal(&mut self, point: [FP; 3]) -> Result<(), String> {
        if !self.game().is_mesh_navigation() || !self.input.mesh_navigation || !self.input.wanted
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
        if self.game().is_mesh_navigation() {
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
