//! One common command owner over the existing managed-input protocol.
use super::{Input, Op, Phase};
use crate::static_adapter::{CommandContext, CommandScope, Consumption, IntentRequest,
    PreparedGameplayCommand, ValidatedGameplay};
use orr_remote::RemoteFrameEvidence;
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
pub(super) fn reply_expired(at: Instant, now: Instant) -> bool {
    now.checked_duration_since(at).is_some_and(|elapsed| elapsed >= std::time::Duration::from_secs(5))
}
pub(super) fn live_timeline(timeline: Option<&orr_bridge::Timeline>, tick: u64) -> bool {
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
pub(in crate::editor) struct GameplayCommandState {
    pub(in crate::editor) contract: ValidatedGameplay,
    pub(in crate::editor) frame: Option<RemoteFrameEvidence>,
    source_anchor: Option<RemoteFrameEvidence>,
    command: Option<PendingCommand>,
    history: VecDeque<(CommandId,CommandStatus)>,
    next_id: u64,
    flight: Option<(u64,Op,Option<J>,bool)>,
    pub(super) state: J,
    scope: Option<(orr_remote::frame_delta::FrameScope,orr_remote::FrameEvidenceOrigin)>,
    scene_epoch: u64,
    last_now: Option<Instant>,
    pub(in crate::editor) error: Option<String>,
    require_new_scope: bool,
    blocked: bool,
}
impl GameplayCommandState {
    pub(in crate::editor) fn new(contract:ValidatedGameplay,state:J,input:&mut Input)->Self {
        input.custom=Some((contract.neutral_value().clone(),None));
        Self {contract,frame:None,source_anchor:None,command:None,history:VecDeque::new(),next_id:1,flight:None,state,
            scope:None,scene_epoch:1,last_now:None,error:None,require_new_scope:false,blocked:false}
    }
    pub(in crate::editor) fn continue_ids_from(&mut self,prior:&Self){self.next_id=prior.next_id;}
    pub(in crate::editor) fn status(&self, id: CommandId) -> Option<&CommandStatus> {
        self.history.iter().find(|(key,_)| *key == id).map(|(_,value)| value)
    }
    pub(in crate::editor) fn apply_state(&mut self, input: &mut Input, state: J) {
        if ["mode","session_mode","epoch","doc_checksum","scene_path"].iter().any(|key|self.state[*key] != state[*key]) { self.cancel(input, "state_scope_changed"); self.frame=None; }
        self.state = state;
    }
    pub(in crate::editor) fn live(&self) -> bool {
        self.state["mode"] == "play" && self.state["playing"] == true
            && self.state["session_mode"] == "record"
            && self.state["head_tick"] == self.state["last_tick"]
            && !self.blocked && self.error.is_none()
            && self.frame.as_ref().is_some_and(|frame| {
                let snapshot = frame.snapshot();
                snapshot.tick() == frame.stamp().tick && snapshot.predicted().tick() == snapshot.tick()
                    && live_timeline(snapshot.timeline(),snapshot.tick())
            })
    }
    pub(in crate::editor) fn can_submit(&self, input: &Input) -> bool {
        self.live() && input.phase == Phase::Active && self.command.is_none() && self.flight.is_none()
    }
    pub(in crate::editor) fn claim(&mut self, input: &mut Input, player: u8) -> Result<(), String> {
        if !self.live() || input.phase != Phase::Off || self.flight.is_some() { return Err("control_not_ready".into()); }
        if player >= self.contract.expected_input().max_players() { return Err("player_out_of_bounds".into()); }
        input.custom = Some((self.contract.neutral_value().clone(), None));
        input.claim(player);
        Ok(())
    }
    pub(in crate::editor) fn submit(&mut self, input: &mut Input, request: IntentRequest) -> Result<CommandId, String> {
        if !self.can_submit(input) { return Err("command_not_ready".into()); }
        let name = request.id().to_owned();
        let intent = self.contract.validate_intent(request)?;
        let evidence = self.frame.as_ref().ok_or("frame_unavailable")?;
        let stamp = evidence.stamp();
        let id = CommandId(self.next_id);
        if id.0 == 0 || id.0 > self.contract.command_max_sequence().unwrap_or(0) { return Err("command_sequence_exhausted".into()); }
        let context = CommandContext { sequence: id.0, player: input.player,
            scope: CommandScope { scene_epoch: self.scene_epoch, play_epoch: stamp.scope.play_epoch } };
        let prepared = self.contract.encode(&intent, context, evidence.snapshot().predicted())?;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0);
        input.custom = Some((prepared.input().clone(), None));
        self.command = Some(PendingCommand { id, metadata: json!({"id":id.0.to_string(),"intent":name,
            "scene_epoch":context.scope.scene_epoch.to_string(),"play_epoch":context.scope.play_epoch.to_string()}),
            prepared, stamp, accepted: None, consumed: None, neutral_accepted: None, sent: false, deadline: self.last_now.ok_or("poll_required")? + std::time::Duration::from_secs(5) });
        self.record(id, CommandStatus::Pending);
        Ok(id)
    }
    pub(in crate::editor) fn record(&mut self, id: CommandId, status: CommandStatus) {
        if let Some((_, value)) = self.history.iter_mut().find(|(key,_)| *key == id) { *value = status; }
        else { if self.history.len() == 64 { self.history.pop_front(); } self.history.push_back((id,status)); }
    }
    pub(in crate::editor) fn cancel(&mut self, input: &mut Input, code: &'static str) {
        if let Some(command) = self.command.take() {
            let message = "Command cancelled; release confirmation or host lease expiry is required".to_owned();
            self.record(command.id, if command.sent { CommandStatus::Uncertain {code,message} }
                else { CommandStatus::Cancelled {code,message} });
        }
        input.release();
        input.custom = Some((self.contract.neutral_value().clone(), None));
    }
    pub(in crate::editor) fn observe(&mut self, input: &mut Input) -> Result<(), String> {
        let (Some(command),Some(evidence)) = (&self.command,&self.frame) else { return Ok(()); };
        let stamp = evidence.stamp();
        if stamp.scope != command.stamp.scope { self.cancel(input, "command_scope_changed"); return Ok(()); }
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
                Consumption::Invalid => self.fail(input, "consumption_invalid"),
                Consumption::Consumed => {
                    let id = command.id;
                    let command = self.command.as_mut().expect("observed command");
                    command.consumed = Some(stamp);
                    command.deadline = self.last_now.expect("poll clock") + std::time::Duration::from_secs(5);
                    input.custom = Some((self.contract.neutral_value().clone(),None));
                    self.record(id,CommandStatus::Consumed{stamp});
                }
            }
        }
        Ok(())
    }

    pub(in crate::editor) fn begin_lifecycle(&mut self,input:&mut Input,new_scope:bool) {
        self.cancel(input,"lifecycle_changed"); self.blocked=true; self.frame=None;
        if new_scope {self.require_new_scope=self.scope.is_some();}
    }
    pub(in crate::editor) fn fail(&mut self,input:&mut Input,code:&'static str) {
        self.flight=None;self.cancel(input,code);self.error=Some(code.into());self.frame=None;
    }
    pub(in crate::editor) fn after_stop(&mut self)->Result<(),String> {
        self.scene_epoch=self.scene_epoch.checked_add(1).ok_or("scene_epoch_exhausted")?;Ok(())
    }
    pub(in crate::editor) fn update(&mut self,input:&mut Input,state:J,evidence:Option<RemoteFrameEvidence>,now:Instant,blocked:bool)->Result<(),String> {
        if self.last_now.is_some_and(|last|now<last) {self.fail(input,"clock_regressed");return Err("clock_regressed".into());}
        self.last_now=Some(now);self.blocked=blocked;self.apply_state(input,state);
        if let Some(evidence) = evidence {
            if evidence.identity() != self.contract.identity() { self.fail(input, "frame_identity_mismatch"); }
            else if self.source_anchor.as_ref().is_some_and(|anchor|!anchor.same_source(&evidence)) {self.fail(input,"frame_source_changed");}
            else {
                if self.source_anchor.is_none(){self.source_anchor=Some(evidence.clone());}
                let scope = (evidence.stamp().scope,evidence.origin());
                if self.scope.is_some_and(|old| old != scope) { self.cancel(input, "frame_scope_changed"); }
                if self.require_new_scope && self.scope == Some(scope) { self.frame = None; }
                else { self.require_new_scope = false; self.scope = Some(scope); self.frame = Some(evidence); }
            }
        } else { if self.command.is_some() { self.cancel(input, "frame_evidence_lost"); } self.frame = None; }
        if self.error.is_some() { self.frame = None; }
        if !self.live() && (self.command.is_some() || matches!(input.phase,Phase::Active | Phase::Claiming)) { self.cancel(input, "not_live"); }
        if self.command.as_ref().is_some_and(|command| now >= command.deadline) { self.cancel(input, "consumption_timeout"); }
        if self.observe(input).is_err() { self.fail(input, "consumption_callback_failed"); }
        if let Some(error)=&self.error {Err(error.clone())}else{Ok(())}
    }
    pub(in crate::editor) fn next(&mut self,input:&mut Input,now:Instant)->Option<(&'static str,J,u64,Op)> {
        let (method,mut params,intent,op)=input.next(now)?;
        let neutral=self.command.as_ref().is_some_and(|command|command.consumed.is_some());
        let metadata=if op==Op::Value && !neutral {self.command.as_ref().map(|command|command.metadata.clone())}else{None};
        if let Some(value)=&metadata {params["command"]=value.clone();}
        self.flight=Some((intent,op,metadata,neutral));
        Some((method,params,intent,op))
    }
    pub(in crate::editor) fn sent(&mut self,op:Op) {if op==Op::Value {if let Some(command)=&mut self.command {command.sent=true;}}}
    pub(in crate::editor) fn answer(&mut self,input:&mut Input,intent:u64,op:Op,result:Result<J,orr_remote::RpcError>,now:Instant) {
        if !self.flight.as_ref().is_some_and(|(key,kind,_,_)|*key==intent && *kind==op) {return;}
        let (_,_,metadata,neutral)=self.flight.take().expect("matched command flight");
        if result.is_err() && metadata.is_some() {
            if let Some(command)=self.command.take() {self.record(command.id,CommandStatus::Rejected{code:"host_rejected",message:"Host rejected this managed input request".into()});}
        }
        if result.is_ok() && metadata.as_ref().is_some_and(|expected|result.as_ref().ok().and_then(|v|v.get("command"))!=Some(expected)) {self.fail(input,"command_ack_mismatch");return;}
        let accepted=result.as_ref().ok().and_then(|v|v["accepted_head_tick"].as_u64());
        if input.answer(intent,op,result,now).is_some() {self.fail(input,"input_ack_invalid");return;}
        if op==Op::Value {if let Some(command)=&mut self.command {if neutral {command.neutral_accepted=accepted;}else{command.accepted=accepted;}}}
    }
    pub(in crate::editor) fn expired_lease(&mut self,input:&mut Input) {self.flight=None;if self.command.is_some(){self.cancel(input,"lease_expired");}}
}
