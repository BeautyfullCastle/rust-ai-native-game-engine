//! One common command owner over the existing managed-input protocol.
use super::{Input, Op, Phase};
use crate::static_adapter::{CommandContext, CommandScope, Consumption, IntentRequest,
    PreparedGameplayCommand, StaticFrameStream, ValidatedGameplay};
use orr_remote::{ErpClient, RemoteConfig, RemoteFrameEvidence, RemoteIdentity, Transport};
use orr_remote::frame_delta::FrameStamp;
use serde_json::{json, Value as J};
use std::collections::VecDeque;
use std::time::Instant;

/// Router-local identity, independent of the managed grant sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandId(pub u64);
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandStatus {
    Pending,
    Consumed { stamp: FrameStamp },
    Completed { consumed: FrameStamp, neutral: FrameStamp },
    Rejected { code: &'static str, message: String },
    Cancelled { code: &'static str, message: String },
    Uncertain { code: &'static str, message: String },
}
fn reply_expired(at: Instant, now: Instant) -> bool {
    now.checked_duration_since(at).is_some_and(|elapsed| elapsed >= std::time::Duration::from_secs(5))
}
fn live_timeline(timeline: Option<&orr_bridge::Timeline>, tick: u64) -> bool {
    timeline.is_some_and(|timeline| timeline.mode == orr_bridge::PlayMode::Record
        && timeline.playing && timeline.tick == timeline.last_tick && timeline.tick == tick)
}
struct PendingCommand {
    id: CommandId,
    metadata: J,
    prepared: PreparedGameplayCommand,
    stamp: FrameStamp,
    accepted: Option<u64>,
    consumed: Option<FrameStamp>,
    neutral_accepted: Option<u64>,
    sent: bool,
    deadline: Instant,
}
struct Flight { rpc: u64, intent: u64, op: Op, metadata: Option<J>, neutral: bool }
#[derive(Clone, Copy)]
enum Lifecycle { Start, StartPlay, Play, Pause, Step, Stop }

/// Owns both real connections. Startup discovery may block; `poll` never waits.
/// This CPU owner does not install a viewport or discover project manifests.
pub struct GameplayRouter {
    contract: ValidatedGameplay,
    client: ErpClient,
    stream: Box<dyn StaticFrameStream>,
    input: Input,
    frame: Option<RemoteFrameEvidence>,
    command: Option<PendingCommand>,
    history: VecDeque<(CommandId, CommandStatus)>,
    next_id: u64,
    flight: Option<Flight>,
    lifecycle: Option<(u64, Lifecycle, Instant)>,
    queued_lifecycle: Option<Lifecycle>,
    cleanup_notice: Option<&'static str>,
    state_request: Option<(u64, Instant)>,
    state: J,
    scope: Option<(orr_remote::frame_delta::FrameScope, orr_remote::FrameEvidenceOrigin)>,
    scene_epoch: u64,
    last_now: Option<Instant>,
    error: Option<String>,
    require_new_scope: bool,
}
impl GameplayRouter {
    /// Both channels originate from this exact local host. Remote pairing is not supported.
    pub fn connect_local(contract: ValidatedGameplay, host: &orr_remote::LocalHost, label: &str,
        caps: orr_remote::Caps, config: RemoteConfig) -> Result<Self, String> {
        if config.token.is_some() { return Err("local_transport_already_authorized".into()); }
        let connector = host.connector();
        let control = connector.connect(label,caps).map_err(|_|"local_control_connect_failed".to_string())?;
        let read = orr_remote::Caps::parse("read").map_err(|_|"read_capability_unavailable".to_string())?;
        let frames = connector.connect("gameplay-frames",read).map_err(|_|"local_frames_connect_failed".to_string())?;
        Self::connect(contract,Box::new(control),Box::new(frames),config)
    }
    fn connect(contract: ValidatedGameplay, control: Box<dyn Transport>, frames: Box<dyn Transport>, mut config: RemoteConfig) -> Result<Self, String> {
        if contract.command_max_sequence().is_none() { return Err("command_capability_required".into()); }
        let mut client = ErpClient::with_transport(control);
        client.call_timeout = config.connect_timeout;
        if let Some(token) = &config.token { client.call("auth",json!({"token":token})).map_err(|_|"control_auth_failed".to_string())?; }
        let discovery = client.call("rpc.discover", J::Null).map_err(|e| e.to_string())?;
        let schema = client.call("registry.schema", J::Null).map_err(|e| e.to_string())?;
        let actual = RemoteIdentity::from_discovery(&discovery, schema.get("schema").cloned().ok_or("missing_schema")?)?;
        if &actual != contract.identity() { return Err("control_identity_mismatch".into()); }
        let input = client.call("registry.input", J::Null).map_err(|e| e.to_string())?;
        contract.expected_input().validate(&input)?;
        let state = client.call("sim.state", J::Null).map_err(|e| e.to_string())?;
        config.expected_identity = Some(contract.identity().clone());
        config.source = "sim".into();
        let stream = (contract.stream_factory())(frames, config, contract.expected_input().clone())?;
        let mut managed = Input::new(true);
        managed.custom = Some((contract.neutral_value().clone(), None));
        Ok(Self { contract, client, stream, input: managed, frame: None, command: None,
            history: VecDeque::new(), next_id: 1, flight: None, lifecycle: None, queued_lifecycle: None, cleanup_notice: None,
            state_request: None, state, scope: None, scene_epoch: 1, last_now: None, error: None, require_new_scope: false })
    }
    pub fn frame_evidence(&self) -> Option<&RemoteFrameEvidence> { self.frame.as_ref() }
    pub fn state(&self) -> &J { &self.state }
    pub fn cleanup_notice(&self) -> Option<&'static str> { self.cleanup_notice }
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
    pub fn input_phase(&self) -> Phase { self.input.phase }
    pub fn status(&self, id: CommandId) -> Option<&CommandStatus> {
        self.history.iter().find(|(key,_)| *key == id).map(|(_,value)| value)
    }
    /// Unknown IDs are never retransmitted; retrying a command is an explicit new submit.
    pub fn retry(&self, id: CommandId) -> Option<&CommandStatus> { self.status(id) }
    fn apply_state(&mut self, state: J) {
        if ["mode","session_mode","epoch","doc_checksum","scene_path"].iter().any(|key|self.state[*key] != state[*key]) { self.cancel("state_scope_changed"); }
        self.state = state;
    }
    fn live(&self) -> bool {
        self.state["mode"] == "play" && self.state["playing"] == true
            && self.state["session_mode"] == "record"
            && self.state["head_tick"] == self.state["last_tick"]
            && self.lifecycle.is_none() && self.queued_lifecycle.is_none() && self.error.is_none()
            && self.frame.as_ref().is_some_and(|frame| {
                let snapshot = frame.snapshot();
                snapshot.tick() == frame.stamp().tick && snapshot.predicted().tick() == snapshot.tick()
                    && live_timeline(snapshot.timeline(),snapshot.tick())
            })
    }
    pub fn can_submit(&self) -> bool {
        self.live() && self.input.phase == Phase::Active && self.command.is_none() && self.flight.is_none()
    }
    pub fn claim(&mut self, player: u8) -> Result<(), String> {
        if !self.live() || self.input.phase != Phase::Off || self.flight.is_some() { return Err("control_not_ready".into()); }
        if player >= self.contract.expected_input().max_players() { return Err("player_out_of_bounds".into()); }
        self.input.custom = Some((self.contract.neutral_value().clone(), None));
        self.input.claim(player);
        Ok(())
    }
    pub fn submit(&mut self, request: IntentRequest) -> Result<CommandId, String> {
        if !self.can_submit() { return Err("command_not_ready".into()); }
        let name = request.id().to_owned();
        let intent = self.contract.validate_intent(request)?;
        let evidence = self.frame.as_ref().ok_or("frame_unavailable")?;
        let stamp = evidence.stamp();
        let id = CommandId(self.next_id);
        if id.0 == 0 || id.0 > self.contract.command_max_sequence().unwrap_or(0) { return Err("command_sequence_exhausted".into()); }
        let context = CommandContext { sequence: id.0, player: self.input.player,
            scope: CommandScope { scene_epoch: self.scene_epoch, play_epoch: stamp.scope.play_epoch } };
        let prepared = self.contract.encode(&intent, context, evidence.snapshot().predicted())?;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0);
        self.input.custom = Some((prepared.input().clone(), None));
        self.command = Some(PendingCommand { id, metadata: json!({"id":id.0.to_string(),"intent":name,
            "scene_epoch":context.scope.scene_epoch.to_string(),"play_epoch":context.scope.play_epoch.to_string()}),
            prepared, stamp, accepted: None, consumed: None, neutral_accepted: None, sent: false, deadline: self.last_now.ok_or("poll_required")? + std::time::Duration::from_secs(5) });
        self.record(id, CommandStatus::Pending);
        Ok(id)
    }
    fn record(&mut self, id: CommandId, status: CommandStatus) {
        if let Some((_, value)) = self.history.iter_mut().find(|(key,_)| *key == id) { *value = status; }
        else { if self.history.len() == 64 { self.history.pop_front(); } self.history.push_back((id,status)); }
    }
    fn cancel(&mut self, code: &'static str) {
        if let Some(command) = self.command.take() {
            let message = "Command cancelled; release confirmation or host lease expiry is required".to_owned();
            self.record(command.id, if command.sent { CommandStatus::Uncertain {code,message} }
                else { CommandStatus::Cancelled {code,message} });
        }
        self.input.release();
        self.input.custom = Some((self.contract.neutral_value().clone(), None));
    }
    pub fn release(&mut self) { self.cancel("control_released"); }
    pub fn focus_lost(&mut self) { self.cancel("focus_lost"); }
    fn lifecycle(&mut self, action: Lifecycle) -> Result<(), String> {
        if self.error.is_some() { return Err("router_failed_reconnect_required".into()); }
        if self.lifecycle.is_some() || self.queued_lifecycle.is_some() { return Err("lifecycle_pending".into()); }
        self.cancel("lifecycle_changed");
        self.state_request = None;
        if matches!(action,Lifecycle::Start | Lifecycle::StartPlay | Lifecycle::Stop) { self.require_new_scope = self.scope.is_some(); }
        self.queued_lifecycle = Some(action);
        self.frame = None;
        Ok(())
    }
    fn post_lifecycle(&mut self, action: Lifecycle) -> Result<(), String> {
        let (method, params) = match action {
            Lifecycle::Start | Lifecycle::StartPlay => ("sim.start", json!({})),
            Lifecycle::Play => ("sim.play", J::Null), Lifecycle::Pause => ("sim.pause", J::Null),
            Lifecycle::Step => ("sim.step", json!({"n":1})), Lifecycle::Stop => ("sim.stop", json!({"include_replay":false})),
        };
        let id = self.client.post(method, params).map_err(|e| e.to_string())?;
        self.lifecycle = Some((id,action,self.last_now.ok_or("poll_required")?));
        Ok(())
    }
    pub fn start(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Start) }
    pub fn play(&mut self) -> Result<(), String> {
        self.lifecycle(if self.state["mode"] == "play" { Lifecycle::Play } else { Lifecycle::StartPlay })
    }
    pub fn pause(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Pause) }
    pub fn step(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Step) }
    pub fn stop(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Stop) }
    fn fail(&mut self, code: &'static str) {
        self.queued_lifecycle = None;
        self.lifecycle = None;
        self.state_request = None;
        self.flight = None;
        self.cancel(code); self.error = Some(code.into()); self.frame = None;
    }
    pub fn poll(&mut self, now: Instant) -> Result<(), String> {
        if self.last_now.is_some_and(|last| now < last) { self.fail("clock_regressed"); return Err("clock_regressed".into()); }
        self.last_now = Some(now);
        if self.lifecycle.is_some_and(|(_,_,at)| reply_expired(at,now)) {
            self.lifecycle = None;
            self.fail("lifecycle_outcome_uncertain");
        }
        if self.state_request.is_some_and(|(_,at)| reply_expired(at,now)) {
            self.state_request = None;
            self.fail("state_outcome_uncertain");
        }
        if self.client.poll().is_err() { self.fail("control_disconnected"); return Err("control_disconnected".into()); }
        while let Some((id,result)) = self.client.responses.pop_front() {
            if self.error.is_some() && !self.flight.as_ref().is_some_and(|flight|flight.rpc == id && flight.op == Op::Release) { continue; }
            if self.lifecycle.is_some_and(|(key,_,_)| key == id) {
                let (_, action, _) = self.lifecycle.take().expect("matched lifecycle");
                match result {
                    Ok(state) => {
                        self.apply_state(state);
                        if matches!(action,Lifecycle::Stop) { self.scene_epoch = self.scene_epoch.checked_add(1).ok_or("scene_epoch_exhausted")?; }
                        if matches!(action,Lifecycle::StartPlay) {
                            let id = self.client.post("sim.play", J::Null).map_err(|e| e.to_string())?;
                            self.lifecycle = Some((id,Lifecycle::Play,now));
                        }
                    }
                    Err(_) => self.fail("lifecycle_rejected"),
                }
            } else if self.state_request.is_some_and(|(key,_)| key == id) {
                self.state_request = None;
                match result { Ok(state) => self.apply_state(state), Err(_) => self.fail("state_unavailable") }
            } else if self.flight.as_ref().is_some_and(|flight| flight.rpc == id) {
                let flight = self.flight.take().expect("matched input");
                if result.is_err() && flight.metadata.is_some() {
                    if let Some(command) = self.command.take() {
                        self.record(command.id,CommandStatus::Rejected { code:"host_rejected",message:"Host rejected this managed input request".into() });
                    }
                }
                if result.is_ok() && flight.metadata.as_ref().is_some_and(|expected| result.as_ref().ok().and_then(|v|v.get("command")) != Some(expected)) {
                    self.fail("command_ack_mismatch"); continue;
                }
                let accepted = result.as_ref().ok().and_then(|v|v["accepted_head_tick"].as_u64());
                let error = self.input.answer(flight.intent,flight.op,result,now);
                if error.is_some() { self.fail("input_ack_invalid"); continue; }
                if flight.op == Op::Value {
                    if let Some(command) = &mut self.command {
                        if flight.neutral { command.neutral_accepted = accepted; } else { command.accepted = accepted; }
                    }
                }
            }
        }
        // Notifications are hints only. Authoritative state and verified stream cuts decide readiness.
        self.client.notifications.clear(); self.client.frames.clear(); self.client.local_frames.clear();
        let _ = self.stream.poll_view();
        if !self.stream.is_alive() || !self.stream.take_errors().is_empty() { self.fail("frame_stream_failed"); }
        let evidence = self.stream.frame_evidence();
        if let Some(evidence) = evidence {
            if evidence.identity() != self.contract.identity() { self.fail("frame_identity_mismatch"); }
            else {
                let scope = (evidence.stamp().scope,evidence.origin());
                if self.scope.is_some_and(|old| old != scope) { self.cancel("frame_scope_changed"); }
                if self.require_new_scope && self.scope == Some(scope) { self.frame = None; }
                else { self.require_new_scope = false; self.scope = Some(scope); self.frame = Some(evidence); }
            }
        } else { if self.command.is_some() { self.cancel("frame_evidence_lost"); } self.frame = None; }
        if self.error.is_some() { self.frame = None; }
        if !self.live() && (self.command.is_some() || matches!(self.input.phase,Phase::Active | Phase::Claiming)) { self.cancel("not_live"); }
        if self.command.as_ref().is_some_and(|command| now >= command.deadline) { self.cancel("consumption_timeout"); }
        if self.observe().is_err() { self.fail("consumption_callback_failed"); }
        if self.error.is_none() && self.state_request.is_none() && self.lifecycle.is_none() && self.queued_lifecycle.is_none() {
            self.state_request = Some((self.client.post("sim.state", J::Null).map_err(|e| e.to_string())?,now));
        }
        if let Some((method,mut params,intent,op)) = self.input.next(now) {
            let neutral = self.command.as_ref().is_some_and(|command| command.consumed.is_some());
            let metadata = if op == Op::Value && !neutral { self.command.as_ref().map(|command|command.metadata.clone()) } else { None };
            if let Some(value) = &metadata { params["command"] = value.clone(); }
            match self.client.post(method,params) {
                Ok(rpc) => { if op == Op::Value { if let Some(command)= &mut self.command { command.sent = true; } }
                    self.flight = Some(Flight{rpc,intent,op,metadata,neutral}); }
                Err(_) => self.fail("input_send_failed"),
            }
        }
        if self.input.phase == Phase::Off {
            if self.command.is_some() { self.cancel("lease_expired"); }
            if self.flight.take().is_some() { self.cleanup_notice = Some("release_unconfirmed_host_lease_expiry_fallback"); }
            if let Some(action) = self.queued_lifecycle.take() { self.post_lifecycle(action)?; }
        }
        if let Some(error) = &self.error { Err(error.clone()) } else { Ok(()) }
    }
    fn observe(&mut self) -> Result<(), String> {
        let (Some(command),Some(evidence)) = (&self.command,&self.frame) else { return Ok(()); };
        let stamp = evidence.stamp();
        if stamp.scope != command.stamp.scope { self.cancel("command_scope_changed"); return Ok(()); }
        if let Some(consumed) = command.consumed {
            if command.neutral_accepted.is_some_and(|tick| stamp.tick > tick) {
                let id = command.id;
                self.command = None;
                self.record(id,CommandStatus::Completed{consumed,neutral:stamp});
            }
        } else if let Some(accepted) = command.accepted {
            let result = self.contract.observe(&command.prepared,command.prepared.context().scope,accepted,evidence.snapshot().predicted())?;
            match result {
                Consumption::Pending => {},
                Consumption::Invalid => self.fail("consumption_invalid"),
                Consumption::Consumed => {
                    let id = command.id;
                    let command = self.command.as_mut().expect("observed command");
                    command.consumed = Some(stamp);
                    command.deadline = self.last_now.expect("poll clock") + std::time::Duration::from_secs(5);
                    self.input.custom = Some((self.contract.neutral_value().clone(),None));
                    self.record(id,CommandStatus::Consumed{stamp});
                }
            }
        }
        Ok(())
    }
}

impl Drop for GameplayRouter {
    fn drop(&mut self) {
        // Best effort only: an unacknowledged claim still relies on host lease expiry.
        self.input.release();
        if let Some((method,params,_,_)) = self.input.next(Instant::now()) {
            let _ = self.client.post(method,params);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn active(now: Instant) -> Input {
        let mut input = Input::new(true);
        input.custom = Some((json!({"button":false}),None));
        input.claim(0);
        let (_,_,intent,op) = input.next(now).unwrap();
        assert!(input.answer(intent,op,Ok(json!({"accepted_head_tick":0,"player":0,"grant":"3","generation":"4","lease_ms":2000})),now).is_none());
        input
    }
    fn ack(input: &mut Input, intent:u64,op:Op, now:Instant) {
        let grant = input.grant.as_ref().unwrap();
        let value = json!({"ok":true,"accepted_head_tick":1,"player":0,"grant":grant.grant,"generation":grant.generation,"sequence":grant.sequence.to_string()});
        assert!(input.answer(intent,op,Ok(value),now).is_none());
    }
    #[test]
    fn reply_deadline_expires_at_five_seconds() {
        let now=Instant::now();
        assert!(!reply_expired(now,now+Duration::from_millis(4999)));
        assert!(reply_expired(now,now+Duration::from_secs(5)));
        assert!(!reply_expired(now+Duration::from_secs(1),now));
    }
    #[test]
    fn paused_evidence_overrules_stale_playing_rpc() {
        let state=json!({"mode":"play","playing":true,"head_tick":4,"last_tick":4});
        let mut timeline=orr_bridge::Timeline { mode:orr_bridge::PlayMode::Record,tick:4,verified_tick:4,
            first_tick:0,last_tick:4,playing:false,speed:Default::default(),checksum:0,
            keyframes:std::sync::Arc::from([]),recent_checksums:std::sync::Arc::from([]),pending_edits:0,branches:0,epoch:0 };
        assert_eq!(state["playing"],true);
        assert!(!live_timeline(Some(&timeline),4));
        timeline.playing=true; assert!(live_timeline(Some(&timeline),4));
        timeline.mode=orr_bridge::PlayMode::Viewer; assert!(!live_timeline(Some(&timeline),4));
        timeline.mode=orr_bridge::PlayMode::Record; timeline.last_tick=5; assert!(!live_timeline(Some(&timeline),4));
        timeline.last_tick=4; assert!(!live_timeline(Some(&timeline),3));
        assert!(!live_timeline(None,4));
    }
    #[test]
    fn custom_values_use_existing_grant_sequence_and_neutral() {
        let now=Instant::now(); let mut input=active(now);
        let (method,value,intent,op)=input.next(now).unwrap();
        assert_eq!(method,"sim.input_value"); assert_eq!(value["value"],json!({"button":false})); assert_eq!(value["sequence"],"1");
        ack(&mut input,intent,op,now); assert!(input.next(now).is_none());
        input.custom=Some((json!({"button":true}),None));
        let (_,value,intent,op)=input.next(now).unwrap(); assert_eq!(value["sequence"],"2"); assert_eq!(value["value"]["button"],true);
        ack(&mut input,intent,op,now);
        input.custom=Some((json!({"button":false}),None));
        let (_,value,_,_)=input.next(now).unwrap(); assert_eq!(value["value"]["button"],false);
    }
    #[test]
    fn custom_mode_keeps_renew_and_expiry_deadlines() {
        let now=Instant::now(); let mut input=active(now);
        let (_,_,intent,op)=input.next(now).unwrap(); ack(&mut input,intent,op,now);
        let (method,_,_,_)=input.next(now+Duration::from_millis(500)).unwrap(); assert_eq!(method,"sim.input_renew");
        let _=input.next(now+Duration::from_millis(2501)); assert_ne!(input.phase,Phase::Active);
    }
    #[test]
    fn custom_release_reuses_managed_release() {
        let now=Instant::now(); let mut input=active(now); input.release();
        let (method,value,intent,op)=input.next(now).unwrap(); assert_eq!(method,"sim.input_release"); assert!(value.get("value").is_none());
        ack(&mut input,intent,op,now); assert_eq!(input.phase,Phase::Off);
    }
}
