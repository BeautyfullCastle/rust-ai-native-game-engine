//! Standalone CPU facade over the same command core used by the editor.
use super::{Input,Op,Phase};
use super::command::{GameplayCommandState,reply_expired};
pub use super::command::{CommandId,CommandStatus};
#[cfg(test)]
use super::command::live_timeline;
use crate::static_adapter::{IntentRequest,StaticFrameStream,ValidatedGameplay};
use orr_remote::{ErpClient,RemoteConfig,RemoteFrameEvidence,RemoteIdentity,Transport};
use serde_json::{json,Value as J};
use std::time::Instant;
struct Flight { rpc:u64,intent:u64,op:Op }
#[derive(Clone, Copy)]
enum Lifecycle { Start, StartPlay, Play, Pause, Step, Stop }

/// Owns both real connections. Startup discovery may block; `poll` never waits.
/// This CPU owner does not install a viewport or discover project manifests.
pub struct GameplayRouter {
    core: GameplayCommandState,
    client: ErpClient,
    stream: Box<dyn StaticFrameStream>,
    input: Input,
    flight: Option<Flight>,
    lifecycle: Option<(u64,Lifecycle,Instant)>,
    queued_lifecycle: Option<Lifecycle>,
    state_request: Option<(u64,Instant)>,
    cleanup_notice: Option<&'static str>,
    last_now: Option<Instant>,
}
impl GameplayRouter {
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
        let mut input=Input::new(true);
        let core=GameplayCommandState::new(contract,state,&mut input);
        Ok(Self{core,client,stream,input,flight:None,lifecycle:None,queued_lifecycle:None,state_request:None,cleanup_notice:None,last_now:None})
    }
    pub fn frame_evidence(&self)->Option<&RemoteFrameEvidence>{self.core.frame.as_ref()}
    pub fn state(&self)->&J{&self.core.state}
    pub fn cleanup_notice(&self)->Option<&'static str>{self.cleanup_notice}
    pub fn error(&self)->Option<&str>{self.core.error.as_deref()}
    pub fn input_phase(&self)->Phase{self.input.phase}
    pub fn status(&self,id:CommandId)->Option<&CommandStatus>{self.core.status(id)}
    pub fn retry(&self,id:CommandId)->Option<&CommandStatus>{self.status(id)}
    pub fn can_submit(&self)->bool{self.core.can_submit(&self.input)}
    pub fn claim(&mut self,player:u8)->Result<(),String>{self.core.claim(&mut self.input,player)}
    pub fn submit(&mut self,request:IntentRequest)->Result<CommandId,String>{self.core.submit(&mut self.input,request)}
    pub fn release(&mut self){self.core.cancel(&mut self.input,"control_released");}
    pub fn focus_lost(&mut self){self.core.cancel(&mut self.input,"focus_lost");}
    fn lifecycle(&mut self, action: Lifecycle) -> Result<(), String> {
        if self.core.error.is_some() { return Err("router_failed_reconnect_required".into()); }
        if self.lifecycle.is_some() || self.queued_lifecycle.is_some() { return Err("lifecycle_pending".into()); }
        self.core.begin_lifecycle(&mut self.input,matches!(action,Lifecycle::Start|Lifecycle::StartPlay|Lifecycle::Stop));
        self.state_request=None;
        self.queued_lifecycle = Some(action);

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
        self.lifecycle(if self.core.state["mode"] == "play" { Lifecycle::Play } else { Lifecycle::StartPlay })
    }
    pub fn pause(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Pause) }
    pub fn step(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Step) }
    pub fn stop(&mut self) -> Result<(), String> { self.lifecycle(Lifecycle::Stop) }
    fn fail(&mut self, code: &'static str) {
        self.queued_lifecycle = None;
        self.lifecycle = None;
        self.state_request = None;
        self.flight = None;
        self.core.fail(&mut self.input,code);
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
            if self.core.error.is_some() && !self.flight.as_ref().is_some_and(|flight|flight.rpc == id && flight.op == Op::Release) { continue; }
            if self.lifecycle.is_some_and(|(key,_,_)| key == id) {
                let (_, action, _) = self.lifecycle.take().expect("matched lifecycle");
                match result {
                    Ok(state) => {
                        self.core.apply_state(&mut self.input,state);
                        if matches!(action,Lifecycle::Stop) { self.core.after_stop()?; }
                        if matches!(action,Lifecycle::StartPlay) {
                            let id = self.client.post("sim.play", J::Null).map_err(|e| e.to_string())?;
                            self.lifecycle = Some((id,Lifecycle::Play,now));
                        }
                    }
                    Err(_) => self.fail("lifecycle_rejected"),
                }
            } else if self.state_request.is_some_and(|(key,_)| key == id) {
                self.state_request = None;
                match result { Ok(state) => self.core.apply_state(&mut self.input,state), Err(_) => self.fail("state_unavailable") }
            } else if self.flight.as_ref().is_some_and(|flight| flight.rpc == id) {
                let flight = self.flight.take().expect("matched input");
                self.core.answer(&mut self.input,flight.intent,flight.op,result,now);
            }
        }
        // Notifications are hints only. Authoritative state and verified stream cuts decide readiness.
        self.client.notifications.clear(); self.client.frames.clear(); self.client.local_frames.clear();
        let _ = self.stream.poll_view();
        if !self.stream.is_alive() || !self.stream.take_errors().is_empty() { self.fail("frame_stream_failed"); }
        let blocked=self.lifecycle.is_some()||self.queued_lifecycle.is_some();
        let _=self.core.update(&mut self.input,self.core.state.clone(),self.stream.frame_evidence(),now,blocked);
        if self.core.error.is_some(){self.lifecycle=None;self.queued_lifecycle=None;self.state_request=None;}
        if self.core.error.is_none() && self.state_request.is_none() && self.lifecycle.is_none() && self.queued_lifecycle.is_none() {
            self.state_request = Some((self.client.post("sim.state", J::Null).map_err(|e| e.to_string())?,now));
        }
        if let Some((method,params,intent,op))=self.core.next(&mut self.input,now) {
            self.core.sent(op);
            match self.client.post(method,params) {
                Ok(rpc)=>{self.flight=Some(Flight{rpc,intent,op});},
                Err(_)=>self.fail("input_send_failed"),
            }
        }
        if self.input.phase == Phase::Off {
            self.core.expired_lease(&mut self.input);
            if self.flight.take().is_some() { self.cleanup_notice = Some("release_unconfirmed_host_lease_expiry_fallback"); }
            if let Some(action) = self.queued_lifecycle.take() { self.post_lifecycle(action)?; }
        }
        if let Some(error) = &self.core.error { Err(error.clone()) } else { Ok(()) }
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
