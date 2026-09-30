// Wall-clock timing of a test run only; nothing here feeds the simulation.
#![allow(clippy::disallowed_types)]

mod common;

use std::time::Instant;

use common::*;
use orr_edit::{
    Check, Cmp, EditError, EditorDoc, MetricStat, MetricValue, Op, Origin, PlayController, ProposalId, ReflectMetrics, VerifyInputs,
    VerifyOptions, VerifyReport,
};
use orr_reflect::Value;
use orr_sample::physics_game::{PhysGame, PhysInput, PhysMetrics};
use orr_session::ControlOp;
use orr_sim::PlayerSlot;

const BODY: &str = "orr_physics::Body";

fn agent() -> Origin {
    Origin::Agent("bot".into())
}

fn set(guid: &orr_reflect::Guid, component: &str, path: &str, value: Value) -> Op {
    Op::SetField { guid: guid.clone(), component: component.into(), path: path.into(), value }
}

/// A deterministic script: the two paddles sweep in opposite directions.
fn script(tick: u64, slot: PlayerSlot) -> PhysInput {
    let dir = if slot.0 == 0 { 1 } else { -1 };
    let phase = ((tick / 30) % 3) as i32 - 1;
    PhysInput::new(dir * phase, 0, 0, tick % 90 == 0)
}

fn scripted(ticks: u32) -> VerifyInputs<'static, PhysGame> {
    VerifyInputs::scripted(ticks, 2, script)
}

fn opts() -> VerifyOptions {
    VerifyOptions { sample_every: 30, ..VerifyOptions::default() }
}

fn propose_move(doc: &mut EditorDoc, name: &str, pos: Value) -> ProposalId {
    let g = guid_named(doc, name);
    let id = doc.propose("move", agent()).unwrap();
    doc.proposal_apply(id, set(&g, BODY, "pos", pos)).unwrap();
    id
}

fn verify(doc: &EditorDoc, id: ProposalId, inputs: &VerifyInputs<'_, PhysGame>) -> VerifyReport {
    doc.verify_proposal::<PhysGame>(id, inputs, &PhysMetrics, &opts()).unwrap()
}

#[test]
fn an_identical_candidate_does_not_diverge() {
    let mut doc = demo_doc();
    let id = doc.propose("nothing", agent()).unwrap();
    let r = verify(&doc, id, &scripted(120));
    assert!(r.identical());
    assert_eq!(r.first_divergence, None);
    assert_eq!(r.first_metric_difference, None);
    assert_eq!((r.start_tick, r.end_tick, r.ticks), (0, 120, 120));
    assert_eq!(r.base_final_checksum, r.candidate_final_checksum);
    assert_ne!(r.base_final_checksum, r.base_start_checksum, "the world moved");
    assert!(!r.metrics.is_empty());
    for m in &r.metrics {
        assert_eq!(m.base, m.candidate, "{}", m.name);
        assert_eq!(m.delta, m.delta.zero_like(), "{}", m.name);
    }
    assert_eq!(r.samples.iter().map(|s| s.tick).collect::<Vec<_>>(), vec![0, 30, 60, 90, 120]);
    assert!(r.samples.iter().all(|s| s.base == s.candidate));
    // The doc against itself gives the same report.
    let again = doc.verify_self::<PhysGame>(&scripted(120), &PhysMetrics, &opts()).unwrap();
    assert_eq!(r, again);
    assert!(r.recording.is_none());
    // PhysMetrics reports what the demo scene has.
    assert_eq!(r.metric("dynamic_bodies").unwrap().base.start, MetricValue::Int(40));
}

#[test]
fn a_moved_body_diverges_and_the_metrics_differ() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 35));
    let r = verify(&doc, id, &scripted(120));
    assert!(!r.identical());
    assert_eq!(r.first_divergence, Some(0), "the initial frames already differ");
    assert_ne!(r.base_start_checksum, r.candidate_start_checksum);
    assert_ne!(r.base_final_checksum, r.candidate_final_checksum);
    assert_eq!(r.first_metric_difference, Some(0));
    let h = r.metric("max_height").unwrap();
    assert_eq!(h.candidate.start, MetricValue::Fixed(orr_fp::FP::from_int(35)));
    assert_ne!(h.candidate.start, h.base.start);
    let mean = r.metric("mean_height").unwrap();
    assert_ne!(mean.base.series, mean.candidate.series);
    // Stats are consistent with the series.
    for m in &r.metrics {
        for st in [&m.base, &m.candidate] {
            assert!(st.series.iter().all(|s| s.1.cmp_value(st.min).is_ge() && s.1.cmp_value(st.max).is_le()));
            assert_eq!(st.start, st.series[0].1);
            assert_eq!(st.end, st.series.last().unwrap().1);
        }
        assert_eq!(m.delta, m.candidate.end.delta(m.base.end));
    }
    assert!(r.lines().iter().any(|l| l.contains("first checksum divergence at tick 0")));
    // The document and the proposal were not touched.
    assert!(doc.history().is_empty());
    assert_eq!(doc.proposal_ops(id).unwrap().len(), 1);
}

#[test]
fn verify_is_deterministic_and_the_same_on_one_thread_or_two() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 35));
    let a = verify(&doc, id, &scripted(150));
    let b = verify(&doc, id, &scripted(150));
    assert_eq!(a, b);
    let seq = VerifyOptions { parallel: false, ..opts() };
    let c = doc.verify_proposal::<PhysGame>(id, &scripted(150), &PhysMetrics, &seq).unwrap();
    assert_eq!(a, c);
    // A shorter run is a prefix of the checksum series.
    let short = doc
        .verify_proposal::<PhysGame>(id, &scripted(150), &PhysMetrics, &VerifyOptions { max_ticks: Some(60), ..opts() })
        .unwrap();
    assert_eq!(short.ticks, 60);
    assert_eq!(short.samples.last(), a.samples.iter().find(|s| s.tick == 60));
}

/// Records a play of `ticks` ticks of `script` from the doc's frame.
fn record(doc: &EditorDoc, ticks: u64) -> orr_edit::StoppedPlay {
    let mut pc = PlayController::<PhysGame>::start_play(doc, doc.play_config(2, 60)).unwrap();
    for t in 1..=ticks {
        for s in 0..2u8 {
            pc.session_mut().set_input(PlayerSlot(s), script(t, PlayerSlot(s)));
        }
        pc.control(ControlOp::Step(1));
    }
    pc.stop_play()
}

#[test]
fn verify_from_a_recording_equals_verify_from_the_same_script() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 35));
    let stopped = record(&doc, 120);
    assert_eq!(stopped.tick, 120);

    let from_replay = verify(&doc, id, &VerifyInputs::from_stopped(&stopped).unwrap());
    let from_script = verify(&doc, id, &scripted(120));
    let rec = from_replay.recording.expect("a recorded run reports the recording check");
    assert_eq!(rec.mismatches, 0, "the base frame reproduces the recording");
    assert!(rec.checked >= 100, "{rec:?}");
    let mut stripped = from_replay.clone();
    stripped.recording = None;
    assert_eq!(stripped, from_script);
    assert_eq!(from_replay.base_final_checksum, stopped.checksum, "the base end state is the recorded end state");

    // The same holds from a saved file's bytes.
    let inputs = VerifyInputs::<PhysGame>::from_replay(&stopped.replay).unwrap();
    assert_eq!(verify(&doc, id, &inputs), from_replay);

    // A doc that is not the recording's start does not reproduce it.
    let mut other = demo_doc();
    let g = guid_named(&other, "body_09");
    other.apply(set(&g, BODY, "pos", vec2(2, 30)), Origin::User).unwrap();
    let r = other.verify_self::<PhysGame>(&inputs, &PhysMetrics, &opts()).unwrap();
    assert!(r.recording.unwrap().mismatches > 0);
    assert!(!r.check(&[Check::RecordingMatches]).results[0].passed);
    assert!(from_replay.check(&[Check::RecordingMatches]).passed);
}

#[test]
fn a_recording_with_debug_edits_replays_them() {
    let doc = demo_doc();
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    pc.control(ControlOp::Step(20));
    let g = target_named(&doc, "body_05");
    assert!(pc.set_field(&g, BODY, "vel", vec2(9, 4)).unwrap());
    pc.control(ControlOp::Step(40));
    let stopped = pc.stop_play();
    let inputs = VerifyInputs::<PhysGame>::from_stopped(&stopped).unwrap();
    let r = doc.verify_self::<PhysGame>(&inputs, &PhysMetrics, &opts()).unwrap();
    assert_eq!(r.debug_commands_replayed, 1);
    assert_eq!(r.recording.unwrap().mismatches, 0);
    assert_eq!(r.base_final_checksum, stopped.checksum);
    let skip = VerifyOptions { debug_commands: false, ..opts() };
    let r2 = doc.verify_self::<PhysGame>(&inputs, &PhysMetrics, &skip).unwrap();
    assert_eq!(r2.debug_commands_replayed, 0);
    assert_ne!(r2.base_final_checksum, stopped.checksum);
}

#[test]
fn checks_pass_and_fail_with_reasons() {
    let mut doc = demo_doc();
    let floor = guid_named(&doc, "floor");
    // Removing the floor lets bodies fall out of the world.
    let id = doc.propose("no floor", agent()).unwrap();
    doc.proposal_apply(id, Op::DespawnEntity { guid: floor }).unwrap();
    let o = VerifyOptions { sample_every: 10, ..VerifyOptions::default() };
    let r = doc.verify_proposal::<PhysGame>(id, &scripted(300), &PhysMetrics, &o).unwrap();

    let none_lost = Check::parse("lost_bodies.max == 0").unwrap();
    let base_none_lost = Check::parse("base:lost_bodies.max == 0").unwrap();
    let out = r.check(&[none_lost.clone(), base_none_lost.clone(), Check::parse("dynamic_bodies.delta == 0").unwrap()]);
    assert!(!out.passed);
    assert!(!out.results[0].passed, "{:?}", out.results[0]);
    assert!(out.results[0].reason.starts_with("lost_bodies.max is "), "{:?}", out.results[0]);
    assert!(out.results[1].passed, "the base keeps every body: {:?}", out.results[1]);
    assert!(out.results[2].passed);
    assert_eq!(out.results[0].check, "lost_bodies.max == 0");

    // The same rule passes for a harmless proposal.
    let ok = propose_move(&mut doc, "body_05", vec2(1, 12));
    let r2 = doc.verify_proposal::<PhysGame>(ok, &scripted(300), &PhysMetrics, &o).unwrap();
    assert!(r2.check(&[none_lost, Check::parse("max_height.max <= 60").unwrap(), Check::parse("min_height >= 0").unwrap()]).passed);

    // Divergence rules, and a rule on an unknown metric.
    assert!(!r2.check(&[Check::NoDivergence]).passed);
    assert!(!r2.check(&[Check::NoDivergenceBefore(1)]).passed);
    assert!(r2.check(&[Check::NoDivergenceBefore(0)]).passed);
    let missing = r2.check(&[Check::parse("nope >= 1").unwrap()]);
    assert!(!missing.passed && missing.results[0].reason.contains("not reported"));
    let same = doc.verify_self::<PhysGame>(&scripted(60), &PhysMetrics, &opts()).unwrap();
    assert!(same.check(&[Check::NoDivergence, Check::NoDivergenceBefore(1000)]).passed);
    assert!(same.check(&[]).passed);

    // Builder form equals parsed form; fixed-point bounds compare with ints.
    let built = Check::metric("lost_bodies", MetricStat::Max, Cmp::Eq, MetricValue::Int(0));
    assert_eq!(built, Check::parse("lost_bodies.max == 0").unwrap());
    assert!(Check::parse("max_speed < 0.5").is_ok());
    assert!(r2.check(&[Check::parse("dynamic_bodies >= 39.5").unwrap()]).passed);
}

#[test]
fn check_text_round_trips_and_bad_text_is_rejected() {
    for t in [
        "lost_bodies.max == 0",
        "base:mean_height.final >= 2.5",
        "kinetic_energy.delta <= 10",
        "no_divergence",
        "no_divergence_before 300",
        "recording_matches",
        "components.orr_physics::Body.start != 3",
    ] {
        let c = Check::parse(t).unwrap();
        assert_eq!(c.to_string(), t);
        assert_eq!(Check::parse(&c.to_string()).unwrap(), c);
    }
    assert_eq!(Check::parse("x <= 1").unwrap().to_string(), "x.final <= 1");
    for bad in ["", "x", "x ~ 1", "x <= one", "no_divergence_before soon", "<= 3", "a b c d"] {
        assert!(matches!(Check::parse(bad), Err(EditError::Verify(_))), "{bad}");
    }
    assert_eq!(Check::parse_all(&["no_divergence", "x < 1"]).unwrap().len(), 2);
}

#[test]
fn reflect_metrics_count_entities_and_components() {
    let mut doc = demo_doc();
    let id = doc.propose("add one", agent()).unwrap();
    let tag = Value::Struct(vec![("slot".into(), Value::Int(1))]);
    doc.proposal_apply(id, Op::SpawnEntity { guid: None, name: None, components: vec![("PaddleTag".into(), tag)] }).unwrap();
    let m = ReflectMetrics::new(doc.types());
    let r = doc.verify_proposal::<PhysGame>(id, &scripted(30), &m, &opts()).unwrap();
    assert_eq!(r.metric("entities").unwrap().delta, MetricValue::Int(1));
    assert_eq!(r.metric("components.PaddleTag").unwrap().delta, MetricValue::Int(1));
    assert_eq!(r.metric("components.orr_physics::Body").unwrap().delta, MetricValue::Int(0));
    assert!(r.check(&[Check::parse("entities.delta == 1").unwrap()]).passed);
    // Two metric sets in one.
    let both = (ReflectMetrics::new(doc.types()), PhysMetrics);
    let r = doc.verify_proposal::<PhysGame>(id, &scripted(30), &both, &opts()).unwrap();
    assert!(r.metric("entities").is_some() && r.metric("lost_bodies").is_some());
}

#[test]
fn setup_errors_are_reported() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 35));
    // A recording must start right after the frames' tick.
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    pc.control(ControlOp::Step(10));
    let mut bytes = pc.session().save_replay();
    let inputs = VerifyInputs::<PhysGame>::from_replay(&bytes).unwrap();
    assert!(doc.verify_self::<PhysGame>(&inputs, &PhysMetrics, &opts()).is_ok());
    let mut later = orr_ecs::Frame::from_bytes(doc.frame_registry().clone(), &doc.frame().to_bytes()).unwrap();
    later.set_tick(5);
    let err = orr_edit::verify_frames::<PhysGame>(&later, &later, &inputs, &PhysMetrics, &opts()).unwrap_err();
    assert!(matches!(&err, EditError::Verify(m) if m.contains("starts at tick 1")), "{err}");
    // Frames at different ticks.
    let err = orr_edit::verify_frames::<PhysGame>(&later, doc.frame(), &scripted(5), &PhysMetrics, &opts()).unwrap_err();
    assert!(matches!(err, EditError::Verify(_)));
    // Garbage is not a replay.
    bytes.truncate(8);
    assert!(matches!(VerifyInputs::<PhysGame>::from_replay(&bytes), Err(EditError::Verify(_))));
    // Zero ticks.
    let none = VerifyInputs::<PhysGame>::scripted(0, 2, script);
    assert!(matches!(doc.verify_proposal::<PhysGame>(id, &none, &PhysMetrics, &opts()), Err(EditError::Verify(_))));
    // A proposal that conflicts with the document now cannot be verified.
    let victim = guid_named(&doc, "body_05");
    doc.apply(Op::DespawnEntity { guid: victim }, Origin::User).unwrap();
    assert!(matches!(verify_err(&doc, id), EditError::ProposalConflict { .. }));
    assert!(matches!(
        doc.verify_proposal::<PhysGame>(ProposalId(77), &scripted(5), &PhysMetrics, &opts()),
        Err(EditError::UnknownProposal(77))
    ));
}

fn verify_err(doc: &EditorDoc, id: ProposalId) -> EditError {
    doc.verify_proposal::<PhysGame>(id, &scripted(5), &PhysMetrics, &opts()).unwrap_err()
}

#[test]
fn verify_600_ticks_of_the_demo_scene_is_fast() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 35));
    let inputs = scripted(600);
    let t = Instant::now();
    let par = verify(&doc, id, &inputs);
    let par_time = t.elapsed();
    let seq_opts = VerifyOptions { parallel: false, ..opts() };
    let t = Instant::now();
    let seq = doc.verify_proposal::<PhysGame>(id, &inputs, &PhysMetrics, &seq_opts).unwrap();
    let seq_time = t.elapsed();
    assert_eq!(par, seq);
    println!("verify 600 ticks, 49 entities: parallel {par_time:?}, sequential {seq_time:?}");
    if !cfg!(debug_assertions) {
        assert!(par_time.as_millis() < 1000, "{par_time:?}");
    }
}
