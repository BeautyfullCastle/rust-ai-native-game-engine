//! One bounded asynchronous handoff to the host-paired editor, not another host.
use std::collections::{BTreeMap,VecDeque};
use std::sync::{Arc,Mutex};
use std::sync::atomic::{AtomicU64,Ordering};
use std::time::{Duration,Instant};
use serde_json::{json,Value as J};
use crate::RpcError;
const MAX_RECORDS:usize=64;
const MAX_PENDING:usize=16;
const MAX_CONNECTIONS:usize=16;
const MAX_REPLY:usize=512*1024;
static NEXT_BINDING:AtomicU64=AtomicU64::new(1);
#[derive(Clone,Debug,PartialEq)]
pub enum GameplayControl {Play,Pause,Step(u32),Stop,Claim(u8),Release}
#[derive(Clone,Debug,PartialEq)]
pub enum GameplayOperation {Describe{intent:Option<String>},Control{action:GameplayControl},Submit{intent:String,payload:J},Status{id:u64},Cancel{id:u64}}
impl GameplayOperation {
    fn writes(&self)->bool{matches!(self,Self::Control{..}|Self::Submit{..}|Self::Cancel{..})}
}
#[derive(Clone,Debug)]
pub struct GameplayRequest {
    pub serial:u64,pub connection:u64,pub client:String,pub binding:u64,
    pub scope:Option<String>,pub request_key:Option<u64>,pub operation:GameplayOperation,pub deadline:Instant,
}
struct Record {request:GameplayRequest,token:u64,taken:bool,result:Option<Result<J,RpcError>>}
struct ConnectionState {token:u64,last_key:u64}
struct Shared {
    binding:u64,owner_alive:bool,next_serial:u64,next_connection:u64,records:VecDeque<Record>,
    connections:BTreeMap<u64,ConnectionState>,commands:BTreeMap<u64,u64>,cancelled:VecDeque<u64>,
}
#[derive(Clone)]
pub struct EditorGameplayService {shared:Arc<Mutex<Shared>>}
pub struct EditorGameplayOwner {shared:Arc<Mutex<Shared>>}
fn lock(shared:&Mutex<Shared>)->std::sync::MutexGuard<'_,Shared>{shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)}
fn unavailable()->RpcError{RpcError::state("editor_gameplay_unavailable","The paired editor gameplay owner is unavailable")}
fn busy()->RpcError{RpcError::state("editor_gameplay_busy","The bounded editor gameplay mailbox is busy")}
fn stale()->RpcError{RpcError::state("editor_gameplay_stale","The editor gameplay binding or request key is stale")}
fn expired(taken:bool)->RpcError{RpcError::state(if taken{"editor_gameplay_outcome_uncertain"}else{"editor_gameplay_timeout"},"The editor gameplay request deadline elapsed")}
fn decimal(value:&J,nonzero:bool)->Option<u64>{let text=value.as_str()?;let number=text.parse::<u64>().ok()?;((!nonzero||number!=0)&&number.to_string()==text).then_some(number)}
fn name(text:&str)->bool{!text.is_empty()&&text.len()<=96&&text.bytes().all(|byte|byte.is_ascii_alphanumeric()||b"_-:".contains(&byte))}
fn scope(text:&str)->bool{!text.is_empty()&&text.len()<=256&&text.bytes().all(|byte|byte.is_ascii_graphic())}
struct Parsed {operation:GameplayOperation,binding:Option<u64>,connection:Option<u64>,scope:Option<String>,key:Option<u64>,timeout:Duration}
fn params(method:&str,value:&J)->Result<Parsed,RpcError>{
    let object=value.as_object().ok_or_else(||RpcError::params("Expected a closed gameplay parameter object"))?;
    let required=|key:&str|object.get(key).ok_or_else(||RpcError::params("Missing gameplay parameter"));
    let optional_intent=||->Result<Option<String>,RpcError>{match object.get("intent"){None=>Ok(None),Some(J::String(text))if name(text)=>Ok(Some(text.clone())),_=>Err(RpcError::params("Invalid gameplay intent ID"))}};
    let (operation,fields)=match method {
        "editor.gameplay.describe"=>(GameplayOperation::Describe{intent:optional_intent()?},vec!["intent"]),
        "editor.gameplay.control"=>{
            let action=match required("action")?.as_str(){Some("play")=>GameplayControl::Play,Some("pause")=>GameplayControl::Pause,Some("stop")=>GameplayControl::Stop,Some("release")=>GameplayControl::Release,
                Some("step")=>{let n=object.get("n").unwrap_or(&J::Null);let n=if n.is_null(){1}else{n.as_u64().ok_or_else(||RpcError::params("Invalid gameplay step count"))?};if !(1..=60).contains(&n){return Err(RpcError::params("Gameplay step count must be 1..60"));}GameplayControl::Step(n as u32)},
                Some("claim")=>{let player=object.get("player").map_or(Some(0),J::as_u64).filter(|p|*p<16).ok_or_else(||RpcError::params("Invalid gameplay player"))?;GameplayControl::Claim(player as u8)},_=>return Err(RpcError::params("Unknown gameplay control action"))};
            if object.contains_key("n")&&!matches!(action,GameplayControl::Step(_))||object.contains_key("player")&&!matches!(action,GameplayControl::Claim(_)){return Err(RpcError::params("Unexpected gameplay control parameter"));}
            (GameplayOperation::Control{action},vec!["action","n","player"])
        },
        "editor.gameplay.submit"=>{
            let intent=optional_intent()?.ok_or_else(||RpcError::params("Missing gameplay intent ID"))?;
            let payload=required("payload")?.clone();if serde_json::to_vec(&payload).map_err(|_|RpcError::params("Invalid gameplay payload"))?.len()>64*1024{return Err(RpcError::params("Gameplay payload exceeds 64 KiB"));}
            (GameplayOperation::Submit{intent,payload},vec!["intent","payload"])
        },
        "editor.gameplay.status"|"editor.gameplay.cancel"=>{let id=decimal(required("command_id")?,true).ok_or_else(||RpcError::params("Invalid gameplay command ID"))?;
            (if method.ends_with("status"){GameplayOperation::Status{id}}else{GameplayOperation::Cancel{id}},vec!["command_id"])},
        _=>return Err(RpcError::params("Unknown editor gameplay method")),
    };
    let writes=operation.writes();
    if object.keys().any(|key|!fields.contains(&key.as_str())&&!matches!(key.as_str(),"binding"|"connection"|"scope"|"request_key"|"timeout_ms")){return Err(RpcError::params("Unknown gameplay parameter"));}
    if !writes&&(object.contains_key("scope")||object.contains_key("request_key")){return Err(RpcError::params("Read methods do not accept mutation fields"));}
    let binding=object.get("binding").map(|v|decimal(v,true).ok_or_else(||RpcError::params("Invalid gameplay binding"))).transpose()?;
    if (writes||matches!(operation,GameplayOperation::Status{..}))&&binding.is_none(){return Err(RpcError::params("Missing gameplay binding"));}
    let connection=object.get("connection").map(|v|decimal(v,true).ok_or_else(||RpcError::params("Invalid gameplay connection fence"))).transpose()?;
    if (writes||matches!(operation,GameplayOperation::Status{..}))&&connection.is_none(){return Err(RpcError::params("Missing gameplay connection fence"));}
    let scope=object.get("scope").map(|value|value.as_str().filter(|text|scope(text)).map(str::to_owned).ok_or_else(||RpcError::params("Invalid gameplay scope"))).transpose()?;
    let key=object.get("request_key").map(|v|decimal(v,true).ok_or_else(||RpcError::params("Invalid gameplay request key"))).transpose()?;
    if writes&&(scope.is_none()||key.is_none()){return Err(RpcError::params("Mutations require scope and request_key"));}
    let timeout=object.get("timeout_ms").map_or(Some(5000),J::as_u64).filter(|v|(50..=5000).contains(v)).ok_or_else(||RpcError::params("Gameplay timeout must be 50..5000 ms"))?;
    Ok(Parsed{operation,binding,connection,scope,key,timeout:Duration::from_millis(timeout)})
}
pub(crate) fn safe_activity_params(method:&str,value:&J)->J {
    let mut safe=json!({});
    for key in ["request_key","command_id"]{if decimal(&value[key],true).is_some(){safe[key]=value[key].clone();}}
    if value["intent"].as_str().is_some_and(name){safe["intent"]=value["intent"].clone();}
    if method=="editor.gameplay.control"&&value["action"].as_str().is_some_and(|action|matches!(action,"play"|"pause"|"step"|"stop"|"claim"|"release")){safe["action"]=value["action"].clone();}
    safe
}
pub(crate) fn safe_activity_result(result:&Result<J,RpcError>)->Result<J,RpcError>{
    match result {
        Ok(value)=>{let mut safe=json!({});if decimal(&value["command_id"],true).is_some(){safe["command_id"]=value["command_id"].clone();}
            if value["status"].as_str().is_some_and(|status|matches!(status,"pending"|"consumed"|"completed"|"rejected"|"cancelled"|"uncertain"|"accepted"|"queued")){safe["status"]=value["status"].clone();}Ok(safe)},
        Err(error)=>Err(RpcError::new(error.code,error.kind().filter(|kind|name(kind)).unwrap_or("editor_gameplay_rejected"),"Editor gameplay request failed")),
    }
}
impl EditorGameplayService {
    pub fn pair()->(Self,EditorGameplayOwner){
        let binding=NEXT_BINDING.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|old|old.checked_add(1)).expect("editor gameplay binding exhausted");
        let shared=Arc::new(Mutex::new(Shared{binding,owner_alive:true,next_serial:1,next_connection:1,records:VecDeque::new(),connections:BTreeMap::new(),commands:BTreeMap::new(),cancelled:VecDeque::new()}));
        (Self{shared:shared.clone()},EditorGameplayOwner{shared})
    }
    pub fn discovery(&self)->J{let state=lock(&self.shared);json!({"binding":state.binding.to_string(),"available":state.owner_alive,"max_pending":MAX_PENDING,"max_results":MAX_RECORDS,"timeout_ms":5000,"scope":"host_paired_editor_gameplay"})}
    pub(crate) fn admit(&self,connection:u64,client:&str,method:&str,value:&J,now:Instant)->Result<u64,RpcError>{
        let Parsed{operation,binding,connection:token,scope,key,timeout}=params(method,value)?;
        let mut state=lock(&self.shared);
        if !state.owner_alive{return Err(unavailable());}
        if binding.is_some_and(|binding|binding!=state.binding){return Err(stale());}
        if client.len()>256{return Err(RpcError::params("Gameplay caller name exceeds its bound"));}
        if token.is_some_and(|token|state.connections.get(&connection).is_none_or(|expected|expected.token!=token)){return Err(stale());}
        if let Some(key)=key {
            if let Some(record)=state.records.iter().find(|record|record.request.connection==connection&&state.connections.get(&connection).is_some_and(|current|current.token==record.token)&&record.request.request_key==Some(key)){
                return if record.request.operation==operation&&record.request.scope==scope{Ok(record.request.serial)}else{Err(RpcError::state("editor_gameplay_key_conflict","Request key was already used with different parameters"))};
            }
            if state.connections.get(&connection).is_some_and(|prior|key<=prior.last_key){return Err(stale());}
        }
        if !state.connections.contains_key(&connection)&&state.connections.len()==MAX_CONNECTIONS{return Err(busy());}
        if state.records.iter().filter(|record|record.result.is_none()).count()>=MAX_PENDING{return Err(busy());}
        if let GameplayOperation::Cancel{id}|GameplayOperation::Status{id}=operation {if state.commands.get(&id)!=Some(&connection){return Err(RpcError::new(crate::error::PERMISSION_DENIED,"permission_denied","Only the original caller may inspect or cancel this command"));}}
        if state.records.len()==MAX_RECORDS {
            let index=state.records.iter().position(|record|record.result.is_some()).ok_or_else(busy)?;
            state.records.remove(index);
        }
        if !state.connections.contains_key(&connection) {
            if !matches!(operation,GameplayOperation::Describe{..}){return Err(stale());}
            let token=state.next_connection;state.next_connection=token.checked_add(1).ok_or_else(unavailable)?;
            state.connections.insert(connection,ConnectionState{token,last_key:0});
        }
        if token.is_some_and(|token|state.connections.get(&connection).is_none_or(|expected|expected.token!=token)){return Err(stale());}
        if let Some(key)=key{state.connections.get_mut(&connection).expect("admitted connection").last_key=key;}
        let serial=state.next_serial;state.next_serial=serial.checked_add(1).ok_or_else(unavailable)?;
        let binding=state.binding;
        let token=state.connections.get(&connection).expect("admitted connection").token;
        state.records.push_back(Record{token,request:GameplayRequest{serial,connection,client:client.to_owned(),binding,scope,request_key:key,operation,deadline:now+timeout},taken:false,result:None});
        Ok(serial)
    }
    pub(crate) fn poll_result(&self,serial:u64,now:Instant)->Option<Result<J,RpcError>>{
        let mut state=lock(&self.shared);let alive=state.owner_alive;
        let Some(record)=state.records.iter_mut().find(|record|record.request.serial==serial)else{return Some(Err(RpcError::state("editor_gameplay_outcome_uncertain","The retained request result is no longer available")));};
        if record.result.is_none(){if !alive{record.result=Some(Err(unavailable()));}else if now>=record.request.deadline{record.result=Some(Err(expired(record.taken)));}}
        record.result.clone()
    }
    pub(crate) fn disconnect(&self,connection:u64){
        let mut state=lock(&self.shared);state.connections.remove(&connection);for record in &mut state.records {if record.request.connection==connection&&record.result.is_none(){record.result=Some(Err(RpcError::state("editor_gameplay_disconnected","The original caller disconnected")));}}
        let commands:Vec<_>=state.commands.iter().filter_map(|(id,owner)|(*owner==connection).then_some(*id)).collect();
        for id in commands{state.commands.remove(&id);if state.cancelled.len()<MAX_RECORDS{state.cancelled.push_back(id);}}
    }
}
impl EditorGameplayOwner {
    pub fn binding_id(&self)->u64{lock(&self.shared).binding}
    pub fn connection_alive(&self,connection:u64)->bool{lock(&self.shared).connections.contains_key(&connection)}
    pub fn take_cancellation(&mut self)->Option<u64>{lock(&self.shared).cancelled.pop_front()}
    pub fn is_active(&self,serial:u64)->bool{let state=lock(&self.shared);state.owner_alive&&state.records.iter().any(|record|record.request.serial==serial&&record.result.is_none()&&Instant::now()<record.request.deadline)}
    pub fn take_request(&mut self)->Option<GameplayRequest>{
        let mut state=lock(&self.shared);let now=Instant::now();
        for record in &mut state.records{
            if record.result.is_none()&&now>=record.request.deadline{record.result=Some(Err(expired(record.taken)));}
            if !record.taken&&record.result.is_none(){record.taken=true;return Some(record.request.clone());}
        }
        None
    }
    pub fn complete(&mut self,serial:u64,result:Result<J,String>){
        let mut state=lock(&self.shared);
        let Some(index)=state.records.iter().position(|record|record.request.serial==serial&&record.taken)else{return;};
        let command=result.as_ref().ok().and_then(|value|decimal(&value["command_id"],true)).filter(|_|matches!(state.records[index].request.operation,GameplayOperation::Submit{..}));
        if state.records[index].result.is_some() || Instant::now()>=state.records[index].request.deadline {
            let rejected=state.records[index].result.as_ref().is_none_or(|result|result.is_err());
            if rejected {if let Some(id)=command {if state.cancelled.len()<MAX_RECORDS&&!state.cancelled.contains(&id){state.cancelled.push_back(id);}}}
            if state.records[index].result.is_none(){state.records[index].result=Some(Err(expired(true)));}
            return;
        }
        let result=result.map_err(|reason|{
            const REASONS:&[&str]=&["stale_scope","command_not_ready","control_not_ready","command_unknown","command_not_owned","control_busy","invalid_intent","invalid_payload","player_out_of_bounds","command_sequence_exhausted","lifecycle_pending","control_disconnected","owner_unavailable","command_capability_unavailable","control_owned","control_failed","scope_exhausted","command_not_cancellable","unknown_intent","client_capability_denied","preview_active"];
            let error=RpcError::state("editor_gameplay_rejected","The paired editor rejected this gameplay operation");
            if REASONS.contains(&reason.as_str()){error.with("reason",json!(reason))}else{error}
        });
        let binding=state.binding;let connection=state.records[index].request.connection;
        let token=state.connections.get(&connection).map(|state|state.token);
        let result=result.and_then(|mut value|{let object=value.as_object_mut().ok_or_else(||RpcError::state("editor_gameplay_result_invalid","Editor gameplay result must be an object"))?;
            object.insert("binding".into(),json!(binding.to_string()));object.insert("connection".into(),json!(token.ok_or_else(stale)?.to_string()));Ok(value)});
        let result=result.and_then(|value|if serde_json::to_vec(&value).is_ok_and(|bytes|bytes.len()<=MAX_REPLY){Ok(value)}else{Err(RpcError::state("editor_gameplay_result_too_large","Editor gameplay result exceeds its bound"))});
        if matches!(state.records[index].request.operation,GameplayOperation::Submit{..}){
            if let Some(id)=result.as_ref().ok().and_then(|value|decimal(&value["command_id"],true)){
                if state.commands.len()==MAX_RECORDS{if let Some(old)=state.commands.keys().next().copied(){state.commands.remove(&old);}}
                let connection=state.records[index].request.connection;state.commands.insert(id,connection);
            }
        }
        state.records[index].result=Some(result);
    }
}
impl Drop for EditorGameplayOwner{fn drop(&mut self){lock(&self.shared).owner_alive=false;}}
impl std::fmt::Debug for EditorGameplayService{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("EditorGameplayService").finish_non_exhaustive()}}

#[cfg(test)]
mod tests {
    use super::*;
    fn submit(binding:u64,key:u64)->J{json!({"binding":binding.to_string(),"connection":"1","scope":"scope:1","request_key":key.to_string(),"intent":"act","payload":{}})}
    #[test]fn dedup_exact_conflict_and_eviction_never_reexecute(){
        let (service,mut owner)=EditorGameplayService::pair();let now=Instant::now();let serial=service.admit(7,"agent","editor.gameplay.describe",&json!({}),now).unwrap();owner.take_request().unwrap();owner.complete(serial,Ok(json!({})));let value=submit(owner.binding_id(),1);
        let first=service.admit(7,"agent","editor.gameplay.submit",&value,now).unwrap();
        assert_eq!(service.admit(7,"agent","editor.gameplay.submit",&value,now).unwrap(),first);
        let mut changed=value.clone();changed["payload"]=json!({"x":1});assert_eq!(service.admit(7,"agent","editor.gameplay.submit",&changed,now).unwrap_err().kind(),Some("editor_gameplay_key_conflict"));
        assert_eq!(owner.take_request().unwrap().serial,first);assert!(owner.take_request().is_none());owner.complete(first,Ok(json!({"command_id":"1"})));
        for key in 2..=70{let serial=service.admit(7,"agent","editor.gameplay.submit",&submit(owner.binding_id(),key),now).unwrap();owner.take_request().unwrap();owner.complete(serial,Ok(json!({"command_id":key.to_string()})));}
        assert_eq!(service.admit(7,"agent","editor.gameplay.submit",&value,now).unwrap_err().kind(),Some("editor_gameplay_stale"));
    }
    #[test]fn stale_binding_closed_fields_and_payload_budget(){
        let (service,owner)=EditorGameplayService::pair();let now=Instant::now();let mut value=submit(owner.binding_id()+1,1);
        assert_eq!(service.admit(1,"agent","editor.gameplay.submit",&value,now).unwrap_err().kind(),Some("editor_gameplay_stale"));
        value["binding"]=json!(owner.binding_id().to_string());value["caller"]=json!("user");assert!(service.admit(1,"agent","editor.gameplay.submit",&value,now).is_err());
        value.as_object_mut().unwrap().remove("caller");value["payload"]=json!("x".repeat(64*1024));assert!(service.admit(1,"agent","editor.gameplay.submit",&value,now).is_err());
    }
    #[test]fn expiry_disconnect_and_owner_loss_are_not_success(){
        let (service,mut owner)=EditorGameplayService::pair();let now=Instant::now();let init=service.admit(1,"agent","editor.gameplay.describe",&json!({}),now).unwrap();owner.take_request().unwrap();owner.complete(init,Ok(json!({})));let serial=service.admit(1,"agent","editor.gameplay.submit",&submit(owner.binding_id(),1),now).unwrap();
        owner.take_request().unwrap();assert_eq!(service.poll_result(serial,now+Duration::from_secs(6)).unwrap().unwrap_err().kind(),Some("editor_gameplay_outcome_uncertain"));owner.complete(serial,Ok(json!({"command_id":"1"})));assert!(service.poll_result(serial,now).unwrap().is_err());
        let serial=service.admit(1,"agent","editor.gameplay.submit",&submit(owner.binding_id(),2),now).unwrap();service.disconnect(1);assert!(!owner.is_active(serial));assert!(!owner.connection_alive(1));
        let serial=service.admit(2,"other","editor.gameplay.describe",&json!({}),now).unwrap();drop(owner);assert_eq!(service.poll_result(serial,now).unwrap().unwrap_err().kind(),Some("editor_gameplay_unavailable"));
    }
    #[test]fn discover_docs_match_closed_fields_and_required_parameters(){
        for (name,allowed,required) in [
            ("describe",vec!["binding","connection","intent","timeout_ms"],vec![]),
            ("control",vec!["binding","connection","scope","request_key","action","player","n","timeout_ms"],vec!["binding","connection","scope","request_key","action"]),
            ("submit",vec!["binding","connection","scope","request_key","intent","payload","timeout_ms"],vec!["binding","connection","scope","request_key","intent","payload"]),
            ("status",vec!["binding","connection","command_id","timeout_ms"],vec!["binding","connection","command_id"]),
            ("cancel",vec!["binding","connection","scope","request_key","command_id","timeout_ms"],vec!["binding","connection","scope","request_key","command_id"]),
        ] {
            let full=format!("editor.gameplay.{name}");let doc=crate::methods::METHODS.iter().find(|doc|doc.name==full).unwrap();
            assert_eq!(doc.params.iter().map(|param|param.name).collect::<Vec<_>>(),allowed);
            assert_eq!(doc.params.iter().filter(|param|param.required).map(|param|param.name).collect::<Vec<_>>(),required);
            let mut minimal=json!({"binding":"1","connection":"1","scope":"scope:1","request_key":"1","intent":"act","payload":{},"action":"play","command_id":"1"});
            minimal.as_object_mut().unwrap().retain(|key,_|required.contains(&key.as_str()));
            assert!(params(&full,&minimal).is_ok());
            for key in required {let mut missing=minimal.clone();missing.as_object_mut().unwrap().remove(key);assert!(params(&full,&missing).is_err(),"{full} missing {key}");}
            let mut extra=minimal;extra["unexpected"]=json!(true);assert!(params(&full,&extra).is_err());
        }
    }
    #[test]fn foreign_status_cancel_and_reconnect_replay_are_denied(){
        let (service,mut owner)=EditorGameplayService::pair();let now=Instant::now();
        for connection in [1,2]{let id=service.admit(connection,"agent","editor.gameplay.describe",&json!({}),now).unwrap();owner.take_request().unwrap();owner.complete(id,Ok(json!({})));}
        let id=service.admit(1,"agent","editor.gameplay.submit",&submit(owner.binding_id(),1),now).unwrap();owner.take_request().unwrap();owner.complete(id,Ok(json!({"command_id":"4"})));
        let status=json!({"binding":owner.binding_id().to_string(),"connection":"2","command_id":"4"});
        assert_eq!(service.admit(2,"other","editor.gameplay.status",&status,now).unwrap_err().kind(),Some("permission_denied"));
        let mut cancel=status;cancel["scope"]=json!("scope:1");cancel["request_key"]=json!("1");assert_eq!(service.admit(2,"other","editor.gameplay.cancel",&cancel,now).unwrap_err().kind(),Some("permission_denied"));
        service.disconnect(1);assert_eq!(owner.take_cancellation(),Some(4));
        let id=service.admit(1,"agent","editor.gameplay.describe",&json!({}),now).unwrap();owner.take_request().unwrap();owner.complete(id,Ok(json!({})));
        assert_eq!(service.admit(1,"agent","editor.gameplay.submit",&submit(owner.binding_id(),2),now).unwrap_err().kind(),Some("editor_gameplay_stale"));
    }
    #[test]fn new_activity_path_discards_payload_scope_errors_and_secrets(){
        let value=json!({"request_key":"3","intent":"act","scope":"private-scope","payload":{"token":"private-token"},"action":"play"});
        let safe=safe_activity_params("editor.gameplay.submit",&value);assert_eq!(safe,json!({"request_key":"3","intent":"act"}));
        let result=safe_activity_result(&Err(RpcError::state("bad","private-path-and-token"))).unwrap_err();assert!(!result.message.contains("private"));
    }

    #[test]fn native_install_ack_is_actual_single_and_rejects_existing_connections(){
        fn make_host()->crate::LocalHost {
            crate::LocalHost::spawn::<orr_sample::physics_game::PhysGame>(||{
                let mut types=orr_reflect::TypeRegistry::new();orr_sample::physics_game::register_reflect(&mut types);
                let doc=orr_edit::EditorDoc::from_yaml(include_str!("../../../scenes/physics_demo.scene.yaml"),types,orr_sim::Simulation::<orr_sample::physics_game::PhysGame>::build_registry(),7).map_err(|_|"fixture_scene_invalid".to_string())?;
                let mut config=crate::ServerConfig::new(crate::Auth::DevNoAuth);config.listen=false;Ok((doc,config))
            }).unwrap()
        }
        let host=make_host();let (service,owner)=EditorGameplayService::pair();host.install_editor_gameplay(service).unwrap();
        let mut reader=crate::ErpClient::with_transport(Box::new(host.connector().connect("reader",crate::Caps::of(&[crate::Cap::Read])).unwrap()));
        assert_eq!(reader.call("rpc.discover",json!({})).unwrap()["engine"]["editor_gameplay"]["available"],true);
        let (second,_owner)=EditorGameplayService::pair();assert_eq!(host.install_editor_gameplay(second).unwrap_err(),"editor_gameplay_already_installed");
        drop(owner);assert!(reader.call("editor.gameplay.describe",json!({})).unwrap_err().to_string().contains("unavailable"));
        let other=make_host();let mut reader=crate::ErpClient::with_transport(Box::new(other.connector().connect("reader",crate::Caps::of(&[crate::Cap::Read])).unwrap()));reader.call("rpc.discover",json!({})).unwrap();
        let (service,_owner)=EditorGameplayService::pair();assert_eq!(other.install_editor_gameplay(service).unwrap_err(),"editor_gameplay_connections_already_present");
    }

}
