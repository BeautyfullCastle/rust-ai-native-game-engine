//! Deterministic held-reply tests against a real host. The transport refuses
//! any blocking receive while armed; speed assertions do not depend on CPU
//! load, profile, or a microsecond threshold.
use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;
use orr_remote::{Caps, Incoming, Request, Transport, USER_CLIENT};

const BODY: &str = "orr_physics::Body";

#[derive(Default)]
struct Gate {
    armed: bool,
    hold: Option<&'static str>,
    requests: Vec<(u64, String, J)>,
    held: VecDeque<Incoming>,
    disconnected: bool,
}

struct Gated {
    inner: Box<dyn Transport>,
    gate: Arc<Mutex<Gate>>,
}

impl Transport for Gated {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.gate.lock().unwrap().requests.push((req.id.unwrap(), req.method.clone(), req.params.clone()));
        self.inner.send(req)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        let mut gate = self.gate.lock().unwrap();
        assert!(!gate.armed || timeout.is_zero(), "UI gesture attempted a blocking receive");
        if gate.disconnected {
            return Err(ClientError::Transport("controlled disconnect".into()));
        }
        if gate.hold.is_none() {
            if let Some(message) = gate.held.pop_front() {
                return Ok(Some(message));
            }
        }
        let Some(message) = self.inner.recv(timeout)? else { return Ok(None) };
        if let Incoming::Text(text) = &message {
            let v: J = serde_json::from_str(text).unwrap();
            if let Some(id) = v["id"].as_u64() {
                let method = gate.requests.iter().find(|(n, _, _)| *n == id).map(|(_, m, _)| m.as_str());
                if method.is_some() && method == gate.hold {
                    gate.held.push_back(message);
                    return Ok(None);
                }
            }
        }
        Ok(Some(message))
    }
}

fn editor() -> (Editor, Arc<Mutex<Gate>>) {
    let mut backend = Backend::connect(&HostSpec::local(default_scene_path())).unwrap();
    let transport = backend.host.as_ref().unwrap().connector().connect(USER_CLIENT, Caps::ALL).unwrap();
    let gate = Arc::new(Mutex::new(Gate::default()));
    backend.erp = ErpClient::with_transport(Box::new(Gated { inner: Box::new(transport), gate: gate.clone() }));
    let mut ed = Editor::on_backend(backend).unwrap();
    ed.select_named("body_05");
    ed.sync();
    gate.lock().unwrap().requests.clear();
    gate.lock().unwrap().armed = true;
    (ed, gate)
}

fn pump_until(ed: &mut Editor, mut pred: impl FnMut(&Editor) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !pred(ed) {
        assert!(Instant::now() < end, "gesture failed to settle: {:?}", ed.status());
        ed.pump();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn set(ed: &mut Editor, path: &str, x: i32) -> bool {
    ed.set_field(&Owner::Component(BODY.into()), path, Value::Fixed(FP::from_int(x)))
}

fn requests(gate: &Arc<Mutex<Gate>>) -> Vec<(String, J)> {
    gate.lock().unwrap().requests.iter()
        .filter(|(_, m, _)| m.starts_with("tx.") || m.ends_with(".patch"))
        .map(|(_, m, p)| (m.clone(), p.clone())).collect()
}

fn settle(ed: &mut Editor, gate: &Arc<Mutex<Gate>>) {
    gate.lock().unwrap().hold = None;
    pump_until(ed, |ed| !ed.gesture.busy());
    gate.lock().unwrap().armed = false;
    ed.sync();
}

#[test]
fn held_begin_patch_commit_never_block_input_and_keep_one_undo_entry() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    let sync_samples = ed.diagnostics().sync_erp_wait.total_samples;
    gate.lock().unwrap().hold = Some("tx.begin");
    ed.begin_edit("held drag");
    for n in 1..=1000 { assert!(set(&mut ed, "pos.x", n)); }
    ed.end_edit();
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    assert_eq!(requests(&gate).len(), 1, "no patch before begin acknowledgement");
    assert_eq!(ed.diagnostics().sync_erp_wait.total_samples, sync_samples);
    gate.lock().unwrap().hold = None;
    ed.pump();
    gate.lock().unwrap().hold = Some("world.patch");
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    assert_eq!(requests(&gate).len(), 2);
    assert_eq!(requests(&gate)[1].1["value"], json!(1000));
    assert!(!ed.undo(), "competing mutation is visibly refused");
    assert!(ed.status().unwrap().error);
    gate.lock().unwrap().hold = None;
    ed.pump();
    gate.lock().unwrap().hold = Some("tx.commit");
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    ed.end_edit(); // duplicate End cannot commit twice
    assert_eq!(requests(&gate).len(), 3);
    assert_eq!(ed.diagnostics().sync_erp_wait.total_samples, sync_samples);
    settle(&mut ed, &gate);
    assert_eq!(ed.history().entries.len(), 1);
    assert_eq!(ed.history().entries[0].label, "held drag");
    assert!(ed.undo());
    ed.sync();
    assert_eq!(ed.checksum(), checksum);
}

#[test]
fn cancel_before_begin_ack_sends_only_begin_then_rollback() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    gate.lock().unwrap().hold = Some("tx.begin");
    ed.begin_edit("cancel before reply");
    assert!(set(&mut ed, "pos.x", 99));
    ed.cancel_edit();
    assert!(!set(&mut ed, "pos.x", 100), "remaining drag events cannot become standalone edits");
    ed.end_edit();
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    settle(&mut ed, &gate);
    assert_eq!(requests(&gate).iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(), ["tx.begin", "tx.rollback"]);
    assert!(ed.history().entries.is_empty());
    assert_eq!(ed.checksum(), checksum);
}

#[test]
fn cancel_after_patch_ack_and_late_updates_restores_host_document() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    gate.lock().unwrap().hold = Some("world.patch");
    ed.begin_edit("cancel patch");
    assert!(set(&mut ed, "pos.x", 99));
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    assert!(set(&mut ed, "pos.x", 100));
    ed.cancel_edit();
    assert!(!set(&mut ed, "pos.x", 101));
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(requests(&gate).iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(), ["tx.begin", "world.patch", "tx.rollback"]);
    assert!(ed.history().entries.is_empty());
    assert_eq!(ed.checksum(), checksum);
}

#[test]
fn rapid_gestures_coalesce_separately_and_preserve_undo_order() {
    let (mut ed, gate) = editor();
    gate.lock().unwrap().hold = Some("tx.begin");
    for (label, x) in [("first", 10), ("second", 20), ("third", 30)] {
        ed.begin_edit(label);
        ed.begin_edit(label); // duplicate begin is not another transaction
        assert!(set(&mut ed, "pos.x", x - 1));
        assert!(set(&mut ed, "pos.x", x));
        ed.end_edit();
    }
    assert!(!ed.save(), "save cannot overtake unfinished edits");
    settle(&mut ed, &gate);
    assert_eq!(requests(&gate).iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(),
        ["tx.begin", "world.patch", "tx.commit", "tx.begin", "world.patch", "tx.commit", "tx.begin", "world.patch", "tx.commit"]);
    assert_eq!(ed.history().entries.len(), 3);
    assert_eq!(ed.read_field(&Owner::Component(BODY.into()), "pos.x").unwrap(), Value::Fixed(FP::from_int(30)));
    assert!(ed.undo());
    assert_eq!(ed.read_field(&Owner::Component(BODY.into()), "pos.x").unwrap(), Value::Fixed(FP::from_int(20)));
    assert!(ed.undo());
    assert_eq!(ed.read_field(&Owner::Component(BODY.into()), "pos.x").unwrap(), Value::Fixed(FP::from_int(10)));
}

#[test]
fn rejected_begin_never_sends_patches_or_falls_through_to_standalone_edits() {
    let (mut ed, gate) = editor();
    let mut other = ed.backend.agent_client("transaction-holder").unwrap();
    other.call("tx.begin", json!({"label": "another client"})).unwrap();
    ed.begin_edit("refused");
    assert!(set(&mut ed, "pos.x", 11));
    pump_until(&mut ed, |ed| !ed.gesture.busy());
    assert!(!set(&mut ed, "pos.x", 12));
    ed.end_edit();
    assert_eq!(requests(&gate).iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(), ["tx.begin"]);
    assert!(ed.status().unwrap().error);
    other.call("tx.rollback", J::Null).unwrap();
    settle(&mut ed, &gate);
    assert!(ed.history().entries.is_empty());
}

#[test]
fn rejected_patch_rolls_back_earlier_accepted_patch_and_discards_later_values() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    gate.lock().unwrap().hold = Some("world.patch");
    ed.begin_edit("rejected patch");
    assert!(set(&mut ed, "pos.x", 11));
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    assert!(set(&mut ed, "pos.y", 99_999));
    assert!(set(&mut ed, "angle", 1));
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(requests(&gate).iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(), ["tx.begin", "world.patch", "world.patch", "tx.rollback"]);
    assert!(ed.history().entries.is_empty());
    assert_eq!(ed.checksum(), checksum);
    assert!(ed.log().iter().any(|m| m.error && m.text.contains("pos.y")));
}

#[test]
fn bounded_queue_visibly_refuses_overflow_and_recovers_after_drain() {
    let (mut ed, gate) = editor();
    gate.lock().unwrap().hold = Some("tx.begin");
    for n in 1..=8 {
        ed.begin_edit("queued");
        assert!(set(&mut ed, "pos.x", n));
        ed.end_edit();
    }
    ed.begin_edit("overflow");
    assert!(!set(&mut ed, "pos.x", 99));
    assert!(ed.status().unwrap().error);
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(ed.history().entries.len(), 8);
    assert_eq!(ed.read_field(&Owner::Component(BODY.into()), "pos.x").unwrap(), Value::Fixed(FP::from_int(8)));
    ed.begin_edit("after drain");
    assert!(set(&mut ed, "pos.x", 9));
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(ed.history().entries.len(), 9);
}

#[test]
fn selection_change_cancels_pending_drag_and_does_not_retarget_its_edits() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    gate.lock().unwrap().hold = Some("tx.begin");
    ed.begin_edit("old selection");
    assert!(set(&mut ed, "pos.x", 88));
    assert!(ed.select_named("body_06"));
    assert!(!set(&mut ed, "pos.x", 99));
    ed.end_edit();
    settle(&mut ed, &gate);
    assert!(ed.history().entries.is_empty());
    assert_eq!(ed.checksum(), checksum);
}

#[test]
fn disconnect_clears_pending_gesture_and_restart_never_replays_it() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    gate.lock().unwrap().hold = Some("tx.begin");
    ed.begin_edit("lost connection");
    assert!(set(&mut ed, "pos.x", 99));
    ed.end_edit();
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    // The real local host is still alive: disconnect handling must not wait
    // for it to exit merely to collect a possible panic reason.
    gate.lock().unwrap().disconnected = true;
    ed.pump();
    let reason = ed.down().unwrap().reason.clone();
    ed.pump();
    assert_eq!(ed.down().unwrap().reason, reason, "preserve the original failure while the host is healthy");
    assert!(!ed.in_gesture());
    assert!(ed.restart());
    ed.sync();
    assert!(ed.history().entries.is_empty());
    assert_eq!(ed.checksum(), checksum);
    assert!(ed.down().is_none());
}

#[test]
fn coalescing_preserves_order_for_overlapping_parent_and_child_fields() {
    let (mut ed, gate) = editor();
    let inspected = ed.inspect().cloned();
    gate.lock().unwrap().hold = Some("tx.begin");
    ed.begin_edit("overlapping fields");
    let owner = Owner::Component(BODY.into());
    assert!(ed.set_field(&owner, "pos", Value::Vec2(FPVec2::new(FP::ONE, FP::ONE))));
    assert!(set(&mut ed, "pos.x", 2));
    assert!(ed.set_field(&owner, "pos", Value::Vec2(FPVec2::new(FP::from_int(3), FP::from_int(3)))));
    assert!(set(&mut ed, "pos.x", 4));
    assert_eq!(ed.inspect(), inspected.as_ref(), "pending values never pretend to be host state");
    ed.end_edit();
    settle(&mut ed, &gate);
    let patches: Vec<_> = requests(&gate).into_iter().filter(|(m, _)| m == "world.patch").collect();
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0].1["path"], "pos");
    assert_eq!(patches[1].1["path"], "pos.x");
    assert_eq!(ed.read_field(&owner, "pos").unwrap(), Value::Vec2(FPVec2::new(FP::from_int(4), FP::from_int(3))));
    assert_eq!(ed.history().entries.len(), 1);
}

#[test]
fn removed_target_cannot_turn_late_drag_input_into_an_unrelated_edit() {
    let (mut ed, gate) = editor();
    let target = ed.selection().unwrap().param();
    let mut other = ed.backend.agent_client("remove-target").unwrap();
    other.call("world.despawn", json!({"entity": target})).unwrap();
    ed.begin_edit("removed target");
    assert!(set(&mut ed, "pos.x", 88));
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(ed.history().entries.len(), 1, "only the external deletion is committed");
    assert!(!ed.history().in_tx);
    assert!(ed.selection().is_none());
    assert!(ed.log().iter().any(|m| m.error));
}

#[test]
fn buffered_foreign_history_cannot_orphan_our_acknowledged_transaction() {
    let (mut ed, gate) = editor();
    let mut other = ed.backend.agent_client("prior-transaction").unwrap();
    let history = |ed: &Editor| -> Vec<bool> {
        ed.backend.erp.notifications.iter()
            .filter(|n| n["method"] == "watch.history")
            .filter_map(|n| n["params"]["in_tx"].as_bool()).collect()
    };
    // History broadcasts are throttled. Keep each state until it was actually
    // received, without Editor ingestion, rather than relying on sleeps.
    other.call("tx.begin", json!({"label": "earlier"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !history(&ed).contains(&true) {
        assert!(Instant::now() < deadline, "missing open history notification");
        ed.backend.erp.poll().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    other.call("tx.commit", J::Null).unwrap();
    while !history(&ed).windows(2).any(|w| w == [true, false]) {
        assert!(Instant::now() < deadline, "missing closed history notification");
        ed.backend.erp.poll().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    ed.begin_edit("our drag");
    assert!(set(&mut ed, "pos.x", 42));
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(ed.history().entries.len(), 1);
    assert_eq!(ed.history().entries[0].label, "our drag");
    assert!(!ed.history().in_tx);
    assert!(ed.undo(), "no orphaned transaction blocks later commands");
}

#[test]
fn a_late_local_panic_reason_enriches_the_initial_disconnect_without_waiting() {
    let spec = HostSpec::Local { scene: default_scene_path(), listen: None, debug_hooks: true };
    let mut ed = Editor::start(&spec).unwrap();
    ed.connection_lost("first connection failure");
    assert_eq!(ed.down().unwrap().reason, "disconnected: first connection failure");
    // The host is still healthy; a later failure supplies additional detail.
    ed.backend.erp.call("debug.panic", J::Null).unwrap();
    pump_until(&mut ed, |ed| ed.down().unwrap().reason.contains("debug.panic"));
    assert!(ed.down().unwrap().reason.starts_with("simulation host stopped:"));
}

#[test]
fn observed_transaction_closure_after_begin_ack_discards_unsent_drag_input() {
    let (mut ed, gate) = editor();
    let checksum = ed.checksum();
    ed.begin_edit("closed by host");
    pump_until(&mut ed, |ed| ed.history.in_tx && !ed.pending.values().any(|p| matches!(p.kind, Pend::Gesture(_, _))));
    // Close on the owning connection to exercise the real notification path
    // used by host expiry, without a timer-dependent timeout test.
    ed.backend.erp.post("tx.rollback", J::Null).unwrap();
    pump_until(&mut ed, |ed| !ed.gesture.busy());
    assert!(!set(&mut ed, "pos.x", 77), "late input must not become a standalone edit");
    ed.end_edit();
    settle(&mut ed, &gate);
    assert_eq!(ed.checksum(), checksum);
    assert!(ed.history().entries.is_empty());
    assert!(!ed.history().in_tx);
    assert!(ed.log().iter().any(|m| m.error && m.text.contains("host closed")));
}

#[path = "../../tests/common/arena.rs"]
mod arena_fixture;

#[test]
fn arena_held_replies_coalesce_position_drag_without_blocking_and_cancel() {
    let host = arena_fixture::ArenaHost::start(false);
    let mut backend = Backend::connect(&HostSpec::remote(&host.url, None)).unwrap();
    let gate = Arc::new(Mutex::new(Gate::default()));
    let transport = orr_remote::PumpedWs::connect(&format!("{}/?client=user", host.url), Duration::from_secs(5)).unwrap();
    backend.erp = ErpClient::with_transport(Box::new(Gated { inner: Box::new(transport), gate: gate.clone() }));
    let mut ed = Editor::on_backend(backend).unwrap();
    ed.select_named("hero"); ed.sync();
    let checksum = ed.checksum();
    let samples = ed.diagnostics().sync_erp_wait.total_samples;
    { let mut g = gate.lock().unwrap(); g.requests.clear(); g.armed = true; g.hold = Some("tx.begin"); }
    ed.begin_edit("Arena held position drag");
    let owner = Owner::Component("Position".into());
    for x in 1..=1000 { assert!(ed.set_field(&owner, "pos.x", Value::Fixed(FP::from_int(x)))); }
    ed.end_edit();
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    assert_eq!(requests(&gate).len(), 1);
    assert_eq!(ed.diagnostics().sync_erp_wait.total_samples, samples);
    settle(&mut ed, &gate);
    assert_eq!(ed.history().entries.len(), 1);
    assert_eq!(requests(&gate).iter().map(|(m,_)| m.as_str()).collect::<Vec<_>>(), ["tx.begin","world.patch","tx.commit"]);
    assert_eq!(requests(&gate)[1].1["component"], "Position");
    assert_eq!(requests(&gate)[1].1["value"], 1000);
    assert!(ed.undo()); ed.sync(); assert_eq!(ed.checksum(), checksum);
    { let mut g = gate.lock().unwrap(); g.requests.clear(); g.armed = true; g.hold = Some("tx.begin"); }
    ed.begin_edit("cancel Arena drag");
    assert!(ed.set_field(&owner, "pos.x", Value::Fixed(FP::from_int(50))));
    ed.cancel_edit(); ed.end_edit();
    pump_until(&mut ed, |_| !gate.lock().unwrap().held.is_empty());
    settle(&mut ed, &gate);
    assert_eq!(requests(&gate).iter().map(|(m,_)| m.as_str()).collect::<Vec<_>>(), ["tx.begin","tx.rollback"]);
    assert_eq!(ed.checksum(), checksum);
}
