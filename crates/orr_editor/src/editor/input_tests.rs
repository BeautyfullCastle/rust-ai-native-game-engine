//! Existing held-reply transport with a real Arena host; no blocking UI polls.
use super::*;
use super::input::{Input, Keys, Op, Phase};
use super::gesture_tests::{Gate, Gated};
use std::sync::Mutex;
#[path = "../../tests/common/arena.rs"]
mod arena;

fn fixture() -> (arena::ArenaHost, Editor, Arc<Mutex<Gate>>) {
    let host = arena::ArenaHost::start(false);
    let mut backend = Backend::connect(&HostSpec::remote(&host.url, None)).unwrap();
    let transport = orr_remote::PumpedWs::connect(&format!("{}/?client=user", host.url), Duration::from_secs(5)).unwrap();
    let gate = Arc::new(Mutex::new(Gate::default()));
    backend.erp = ErpClient::with_transport(Box::new(Gated { inner: Box::new(transport), gate: gate.clone() }));
    let mut ed = Editor::on_backend(backend).unwrap();
    ed.play(); ed.sync();
    gate.lock().unwrap().requests.clear();
    gate.lock().unwrap().armed = true;
    (host, ed, gate)
}
fn until(ed: &mut Editor, pred: impl Fn(&Editor) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !pred(ed) { assert!(Instant::now() < end, "input did not settle: {:?}", ed.status()); ed.pump(); std::thread::sleep(Duration::from_millis(1)); }
}
fn methods(g: &Arc<Mutex<Gate>>) -> Vec<String> {
    g.lock().unwrap().requests.iter().filter(|(_,m,_)| m.starts_with("sim.input")).map(|(_,m,_)|m.clone()).collect()
}
#[test]
fn input_focus_alone_never_claims_or_changes_legacy_input() {
    let (_host, mut ed, gate) = fixture();
    for _ in 0..30 { ed.arena_keys(true, Keys { x:1,y:0,fire:true }); ed.pump(); }
    assert!(methods(&gate).is_empty());
    assert_eq!(ed.input_phase(), Phase::Off);
}
#[test]
fn input_late_claim_after_focus_loss_only_releases() {
    let (_host, mut ed, gate) = fixture();
    gate.lock().unwrap().hold = Some("sim.input_claim");
    assert!(ed.take_control(0,true));
    for _ in 0..30 { ed.pump(); }
    ed.arena_keys(false, Keys::default());
    assert_eq!(ed.input_phase(),Phase::Releasing);
    gate.lock().unwrap().hold = None;
    until(&mut ed, |e| e.input_phase() == Phase::Off);
    assert_eq!(methods(&gate),vec!["sim.input_claim","sim.input_release"]);
}
#[test]
fn input_bursts_coalesce_and_release_follows_pending_update() {
    let (host, mut ed, gate) = fixture();
    gate.lock().unwrap().hold = Some("sim.input_value");
    assert!(ed.take_control(0,true));
    until(&mut ed, |e|e.input_phase() == Phase::Active);
    for x in 0..200 { ed.arena_keys(true,Keys { x:(x%3-1) as i8,y:0,fire:true }); ed.pump(); }
    assert_eq!(methods(&gate),vec!["sim.input_claim","sim.input_value"]);
    ed.release_control();
    for _ in 0..10 { ed.pump(); }
    assert_eq!(methods(&gate),vec!["sim.input_claim","sim.input_value","sim.input_release"]);
    assert!(!ed.take_control(0,true),"unresolved prior update prevents new claim accumulation");
    gate.lock().unwrap().hold = None;
    until(&mut ed, |e|e.input_phase() == Phase::Off);
    assert_eq!(methods(&gate),vec!["sim.input_claim","sim.input_value","sim.input_release"]);
    let mut client = host.client();
    assert_eq!(client.call("sim.state",J::Null).unwrap()["managed_held"]["slots"],json!([]));
}
fn claim_ack() -> J { json!({"player":0,"grant":"9","generation":"3","lease_ms":2000,"accepted_head_tick":5}) }
fn active() -> (Input,Instant) {
    let now=Instant::now(); let mut input=Input::new(true); input.claim(0);
    let (_,_,intent,op)=input.next(now).unwrap(); input.answer(intent,op,Ok(claim_ack()),now);
    (input,now)
}
#[test]
fn input_stale_ack_missing_ack_and_lease_timeout_fail_closed() {
    let (mut input,now)=active();
    let (_,params,intent,op)=input.next(now).unwrap();
    assert_eq!(params["sequence"],"1");
    assert!(input.answer(intent,op,Ok(json!({"ok":true,"player":0,"grant":"8","generation":"3","sequence":"1","accepted_head_tick":6})),now).is_some());
    assert_eq!(input.phase,Phase::Off);
    let (mut input,now)=active(); let (_,_,intent,op)=input.next(now).unwrap();
    assert!(input.answer(intent,op,Ok(json!({"ok":true})),now).is_some());
    assert_eq!(input.phase,Phase::Off);
    let (mut input,now)=active(); input.next(now).unwrap();
    assert_eq!(input.next(now+Duration::from_secs(2)).unwrap().0,"sim.input_release");
    input.next(now+Duration::from_secs(4)); assert_eq!(input.phase,Phase::Off);
    // An obsolete reply cannot revive a new intent, even for a reused player.
    input = Input::new(true); input.claim(0); input.release(); input.next(now);
    input.claim(0); input.next(now+Duration::from_secs(3)).unwrap();
    input.answer(1,Op::Claim,Ok(claim_ack()),now+Duration::from_secs(3));
    assert_eq!(input.phase,Phase::Claiming);
}
#[test]
fn input_renew_is_bounded_and_sequences_increase() {
    let (mut input,now)=active();
    let (_,params,intent,op)=input.next(now).unwrap();
    let mut ack=params; ack["ok"]=json!(true); ack["accepted_head_tick"]=json!(6);
    input.answer(intent,op,Ok(ack),now);
    assert!(input.next(now+Duration::from_millis(499)).is_none());
    let (method,params,_,_)=input.next(now+Duration::from_millis(500)).unwrap();
    assert_eq!(method,"sim.input_renew");assert_eq!(params["sequence"],"2");
    for _ in 0..200 { assert!(input.next(now+Duration::from_millis(501)).is_none()); }
}

#[test]
fn input_timeout_keeps_request_fence_and_late_claim_releases() {
    let (_host,mut ed,gate)=fixture();
    gate.lock().unwrap().hold=Some("sim.input_claim");
    assert!(ed.take_control(0,true));
    for _ in 0..30 { ed.pump(); }
    // Advance only the local request deadline, retaining the transport reply.
    let now=Instant::now()+Duration::from_secs(3);
    assert!(ed.input.next(now).is_none());
    for _ in 0..200 { assert!(!ed.take_control(0,true));ed.pump(); }
    assert_eq!(methods(&gate),vec!["sim.input_claim"]);
    assert_eq!(ed.pending.values().filter(|p|matches!(p.kind,Pend::Input(..))).count(),1);
    gate.lock().unwrap().hold=None;
    until(&mut ed,|e|e.input_phase()==Phase::Off && !e.input_cleanup_pending());
    assert_eq!(methods(&gate),vec!["sim.input_claim","sim.input_release"]);
}

#[test]
fn input_old_paused_state_cannot_cancel_newer_claim() {
    let (_host,mut ed,gate)=fixture();
    gate.lock().unwrap().armed=false;
    ed.pause();ed.sync();
    gate.lock().unwrap().hold=Some("sim.state");
    ed.dirty.state=true;ed.pump();
    for _ in 0..30 {ed.pump();std::thread::sleep(Duration::from_millis(1));}
    assert!(!gate.lock().unwrap().held.is_empty());
    ed.play();
    assert!(ed.take_control(0,true));
    until(&mut ed,|e|e.input_phase()==Phase::Active);
    gate.lock().unwrap().armed=true;
    gate.lock().unwrap().hold=None;
    for _ in 0..30 {ed.pump();}
    assert_eq!(ed.input_phase(),Phase::Active);
    assert!(!methods(&gate).contains(&"sim.input_release".to_string()));
}
