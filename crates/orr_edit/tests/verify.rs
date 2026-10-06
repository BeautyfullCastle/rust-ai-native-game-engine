// Wall-clock timing of a test run only; nothing here feeds the simulation.
#![allow(clippy::disallowed_types)]

mod common;

use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use common::*;
use orr_edit::{
    Check, Cmp, EditError, EditorDoc, MetricStat, MetricValue, Op, Origin, PlayController, ProposalId, ReflectMetrics, VerifyInputs,
    VerifyOptions, VerifyReport,
};
use orr_reflect::Value;
use orr_sample::physics_game::{PhysGame, PhysInput, PhysMetrics};
use orr_session::ControlOp;
use orr_sim::{Game, Metrics, PlayerSlot, SimContext, Simulation, System};

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

struct CancelOnSample {
    cancel: Arc<AtomicBool>,
    tick: u64,
}

impl Metrics for CancelOnSample {
    fn sample(&self, frame: &orr_ecs::Frame) -> Vec<(String, MetricValue)> {
        if frame.tick() == self.tick {
            self.cancel.store(true, Ordering::Relaxed);
        }
        Vec::new()
    }
}

struct CancelAfterBothSample {
    cancel: Arc<AtomicBool>,
    tick: u64,
    samples: AtomicUsize,
}

impl Metrics for CancelAfterBothSample {
    fn sample(&self, frame: &orr_ecs::Frame) -> Vec<(String, MetricValue)> {
        if frame.tick() == self.tick && self.samples.fetch_add(1, Ordering::Relaxed) + 1 == 2 {
            self.cancel.store(true, Ordering::Relaxed);
        }
        Vec::new()
    }
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
fn sampling_coverage_uses_actual_boundaries_relative_to_the_start() {
    let doc = demo_doc();
    let mut frame = orr_ecs::Frame::from_bytes(doc.frame_registry().clone(), &doc.frame().to_bytes()).unwrap();
    frame.set_tick(17);
    let mut final_checksum = None;
    for (interval, expected) in [
        (0, vec![17, 22]),
        (1, vec![17, 18, 19, 20, 21, 22]),
        (2, vec![17, 19, 21, 22]),
        (60, vec![17, 22]),
    ] {
        let options = VerifyOptions { sample_every: interval, ..VerifyOptions::default() };
        let r = orr_edit::verify_frames::<PhysGame>(&frame, &frame, &scripted(5), &PhysMetrics, &options).unwrap();
        assert_eq!((r.start_tick, r.end_tick, r.ticks), (17, 22, 5));
        assert_eq!(r.sample_every, interval);
        assert_eq!(r.samples.iter().map(|s| s.tick).collect::<Vec<_>>(), expected);
        assert_eq!(r.every_tick_boundary_observed(), interval == 1);
        assert_eq!(r.lines().iter().any(|line| line.contains("between samples not checked")), interval != 1);
        let check = r.check(&[Check::parse("dynamic_bodies.max == 40").unwrap()]);
        assert!(check.passed);
        assert_eq!(check.results[0].reason.contains("between samples not checked"), interval != 1);
        assert!(r.identical());
        assert_eq!(*final_checksum.get_or_insert(r.base_final_checksum), r.base_final_checksum);
    }
    // One tick has only the two endpoints, regardless of requested interval.
    for interval in [0, 1, 60] {
        let options = VerifyOptions { sample_every: interval, ..VerifyOptions::default() };
        let r = orr_edit::verify_frames::<PhysGame>(&frame, &frame, &scripted(1), &PhysMetrics, &options).unwrap();
        assert_eq!(r.samples.len(), 2);
        assert!(r.every_tick_boundary_observed());
        assert!(!r.lines().iter().any(|line| line.contains("between samples not checked")));
        // Preserve the existing zero-tick contract: no report is produced.
        assert!(matches!(
            orr_edit::verify_frames::<PhysGame>(&frame, &frame, &scripted(0), &PhysMetrics, &options),
            Err(EditError::Verify(message)) if message == "no ticks to run"
        ));
    }
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

#[test]
fn cancellable_verify_stops_before_start_and_between_ticks() {
    let doc = demo_doc();
    let already_cancelled = AtomicBool::new(true);
    let never_run = VerifyInputs::scripted(4, 2, |_, _| panic!("pre-cancelled input must not run"));
    let result = orr_edit::verify_frames_cancellable::<PhysGame>(
        doc.frame(),
        doc.frame(),
        &never_run,
        &PhysMetrics,
        &opts(),
        &already_cancelled,
    );
    assert!(matches!(result, Err(EditError::VerifyCancelled)));

    let cancel = Arc::new(AtomicBool::new(false));
    let input_calls = Arc::new(AtomicUsize::new(0));
    let calls = input_calls.clone();
    let inputs = VerifyInputs::scripted(4, 2, move |tick, slot| {
        calls.fetch_add(1, Ordering::Relaxed);
        script(tick, slot)
    });
    let metrics = CancelOnSample { cancel: cancel.clone(), tick: 1 };
    let sequential = VerifyOptions { parallel: false, sample_every: 1, ..opts() };
    let result = orr_edit::verify_frames_cancellable::<PhysGame>(doc.frame(), doc.frame(), &inputs, &metrics, &sequential, &cancel);
    assert!(matches!(result, Err(EditError::VerifyCancelled)));
    assert_eq!(input_calls.load(Ordering::Relaxed), 2, "only tick 1 inputs were requested");
}

#[test]
fn scripted_input_cancellation_skips_later_players_and_simulation() {
    let doc = demo_doc();
    let checksum = doc.frame().checksum();
    for cancel_slot in 0..3 {
        let cancel = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let inputs = VerifyInputs::scripted(4, 3, |tick, slot| {
            assert_eq!(tick, 1);
            assert_eq!(usize::from(slot.0), calls.fetch_add(1, Ordering::Relaxed));
            if slot.0 == cancel_slot {
                cancel.store(true, Ordering::Relaxed);
            }
            script(tick, slot)
        });
        struct InitialSampleOnly(AtomicUsize);
        impl Metrics for InitialSampleOnly {
            fn sample(&self, frame: &orr_ecs::Frame) -> Vec<(String, MetricValue)> {
                assert_eq!(
                    frame.tick(),
                    0,
                    "cancelled inputs must not reach simulation"
                );
                self.0.fetch_add(1, Ordering::Relaxed);
                Vec::new()
            }
        }
        let metrics = InitialSampleOnly(AtomicUsize::new(0));
        let sequential = VerifyOptions {
            parallel: false,
            sample_every: 1,
            ..opts()
        };
        let result = orr_edit::verify_frames_cancellable::<PhysGame>(
            doc.frame(),
            doc.frame(),
            &inputs,
            &metrics,
            &sequential,
            &cancel,
        );
        assert!(matches!(result, Err(EditError::VerifyCancelled)));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            usize::from(cancel_slot) + 1,
            "no later player callback is invoked"
        );
        assert_eq!(
            metrics.0.load(Ordering::Relaxed),
            1,
            "the candidate side never starts"
        );
        assert_eq!(doc.frame().checksum(), checksum);
    }
}

#[test]
fn uncancelled_scripted_players_preserve_complete_reports() {
    let doc = demo_doc();
    for players in [0, 1, 3] {
        for parallel in [false, true] {
            let options = VerifyOptions {
                parallel,
                sample_every: 1,
                ..opts()
            };
            let expected = orr_edit::verify_frames::<PhysGame>(
                doc.frame(),
                doc.frame(),
                &VerifyInputs::scripted(16, players, script),
                &PhysMetrics,
                &options,
            )
            .unwrap();
            let calls = AtomicUsize::new(0);
            let inputs = VerifyInputs::scripted(16, players, |tick, slot| {
                calls.fetch_add(1, Ordering::Relaxed);
                script(tick, slot)
            });
            let actual = orr_edit::verify_frames_cancellable::<PhysGame>(
                doc.frame(),
                doc.frame(),
                &inputs,
                &PhysMetrics,
                &options,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(actual.ticks, 16);
            assert_eq!(calls.load(Ordering::Relaxed), 2 * 16 * usize::from(players));
        }
    }
}

// The test system requests cancellation during an ordinary successful tick.
// Thread-local instrumentation keeps concurrent tests independent without
// coordinating worker threads; these fixtures deliberately run sequentially.
#[derive(Default)]
struct TickCancellationProbe {
    cancel: Arc<AtomicBool>,
    cancel_tick: Option<u64>,
    ticks: Vec<u64>,
    inputs: Vec<(u64, PlayerSlot)>,
    samples: Vec<u64>,
}

thread_local! {
    static TICK_CANCELLATION: RefCell<TickCancellationProbe> = RefCell::default();
}

struct TickCancellationGame;

impl Game for TickCancellationGame {
    type Input = u8;
    type Command = <PhysGame as Game>::Command;
    type Event = u8;
    type Config = ();

    fn register(_: &mut orr_ecs::ComponentRegistryBuilder) {}
    fn setup(_: &mut orr_ecs::Frame, _: &()) {}
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(cancel_during_tick as fn(&mut SimContext<Self>))]
    }
}

fn cancel_during_tick(ctx: &mut SimContext<TickCancellationGame>) {
    TICK_CANCELLATION.with_borrow_mut(|probe| {
        probe.ticks.push(ctx.tick);
        if probe.cancel_tick == Some(ctx.tick) {
            probe.cancel.store(true, Ordering::Relaxed);
        }
    });
}

fn tick_probe_input(tick: u64, slot: PlayerSlot) -> u8 {
    TICK_CANCELLATION.with_borrow_mut(|probe| probe.inputs.push((tick, slot)));
    0
}

struct TickProbeMetrics;

impl Metrics for TickProbeMetrics {
    fn sample(&self, frame: &orr_ecs::Frame) -> Vec<(String, MetricValue)> {
        TICK_CANCELLATION.with_borrow_mut(|probe| probe.samples.push(frame.tick()));
        vec![("tick".into(), MetricValue::Int(frame.tick() as i64))]
    }
}

#[test]
fn cancellation_during_successful_tick_skips_sampling_and_candidate() {
    let sim = Simulation::<TickCancellationGame>::new((), 60, 1);
    let frame = sim.frame();
    let bytes = frame.to_bytes();
    let checksum = frame.checksum();
    let options = VerifyOptions {
        parallel: false,
        sample_every: 1,
        ..opts()
    };
    for cancel_tick in 1..=3 {
        let cancel = Arc::new(AtomicBool::new(false));
        TICK_CANCELLATION.set(TickCancellationProbe {
            cancel: cancel.clone(),
            cancel_tick: Some(cancel_tick),
            ..Default::default()
        });
        let inputs = VerifyInputs::scripted(3, 2, tick_probe_input);
        let result = orr_edit::verify_frames_cancellable::<TickCancellationGame>(
            frame,
            frame,
            &inputs,
            &TickProbeMetrics,
            &options,
            &cancel,
        );
        let probe = TICK_CANCELLATION.take();
        assert!(matches!(result, Err(EditError::VerifyCancelled)));
        assert_eq!(probe.ticks, (1..=cancel_tick).collect::<Vec<_>>());
        assert_eq!(
            probe.inputs,
            (1..=cancel_tick)
                .flat_map(|t| [(t, PlayerSlot(0)), (t, PlayerSlot(1))])
                .collect::<Vec<_>>()
        );
        assert_eq!(
            probe.samples,
            (0..cancel_tick).collect::<Vec<_>>(),
            "no post-cancel sample or candidate start"
        );
        assert_eq!(frame.to_bytes(), bytes);
        assert_eq!(frame.checksum(), checksum);
    }
}

#[test]
fn uncancelled_successful_ticks_preserve_complete_reports() {
    let sim = Simulation::<TickCancellationGame>::new((), 60, 1);
    let frame = sim.frame();
    let bytes = frame.to_bytes();
    let checksum = frame.checksum();
    let options = VerifyOptions {
        parallel: false,
        sample_every: 1,
        ..opts()
    };
    let inputs = VerifyInputs::scripted(3, 2, tick_probe_input);
    TICK_CANCELLATION.set(TickCancellationProbe::default());
    let expected = orr_edit::verify_frames::<TickCancellationGame>(
        frame,
        frame,
        &inputs,
        &TickProbeMetrics,
        &options,
    )
    .unwrap();
    let expected_probe = TICK_CANCELLATION.take();
    let actual = orr_edit::verify_frames_cancellable::<TickCancellationGame>(
        frame,
        frame,
        &inputs,
        &TickProbeMetrics,
        &options,
        &AtomicBool::new(false),
    )
    .unwrap();
    let actual_probe = TICK_CANCELLATION.take();
    assert_eq!(actual, expected);
    assert_eq!(actual.ticks, 3);
    assert_eq!(actual_probe.ticks, vec![1, 2, 3, 1, 2, 3]);
    assert_eq!(actual_probe.ticks, expected_probe.ticks);
    assert_eq!(
        actual_probe.inputs,
        (1..=3)
            .cycle()
            .take(6)
            .flat_map(|t| [(t, PlayerSlot(0)), (t, PlayerSlot(1))])
            .collect::<Vec<_>>()
    );
    assert_eq!(actual_probe.inputs, expected_probe.inputs);
    assert_eq!(actual_probe.samples, vec![0, 1, 2, 3, 0, 1, 2, 3]);
    assert_eq!(actual_probe.samples, expected_probe.samples);
    assert_eq!(frame.to_bytes(), bytes);
    assert_eq!(frame.checksum(), checksum);
}

#[test]
fn cancellable_verify_does_not_return_a_report_when_cancelled_after_final_tick() {
    let doc = demo_doc();
    let cancel = Arc::new(AtomicBool::new(false));
    let input_calls = Arc::new(AtomicUsize::new(0));
    let calls = input_calls.clone();
    let inputs = VerifyInputs::scripted(4, 2, move |tick, slot| {
        calls.fetch_add(1, Ordering::Relaxed);
        script(tick, slot)
    });
    let metrics = CancelOnSample { cancel: cancel.clone(), tick: 4 };
    let sequential = VerifyOptions { parallel: false, sample_every: 4, ..opts() };
    let result = orr_edit::verify_frames_cancellable::<PhysGame>(doc.frame(), doc.frame(), &inputs, &metrics, &sequential, &cancel);
    assert!(matches!(result, Err(EditError::VerifyCancelled)));
    assert_eq!(input_calls.load(Ordering::Relaxed), 8, "all four ticks ran before the final sample cancelled");
}

#[test]
fn parallel_cancellation_is_shared_by_and_joins_both_sides() {
    let doc = demo_doc();
    let cancel = Arc::new(AtomicBool::new(false));
    let inputs = scripted(16);
    let metrics = CancelAfterBothSample { cancel: cancel.clone(), tick: 1, samples: AtomicUsize::new(0) };
    let parallel = VerifyOptions { parallel: true, sample_every: 1, ..opts() };
    let result = orr_edit::verify_frames_cancellable::<PhysGame>(doc.frame(), doc.frame(), &inputs, &metrics, &parallel, &cancel);
    assert!(matches!(result, Err(EditError::VerifyCancelled)));
    assert_eq!(metrics.samples.load(Ordering::Relaxed), 2, "both sides reached the same sampled tick");
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

#[test]
fn verified_accept_refuses_document_changes_then_accepts_a_new_verification() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 12));
    let expected = doc.proposal_state(id).unwrap();
    let report = verify(&doc, id, &scripted(1));
    let other = guid_named(&doc, "body_06");
    doc.apply(set(&other, BODY, "pos", vec2(2, 14)), Origin::User).unwrap();
    assert!(doc.proposal_check(id).is_ok(), "the ops still apply: ordinary conflict detection is insufficient");
    let unchanged = state(&doc);
    let history = doc.history();
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));
    assert_eq!(state(&doc), unchanged);
    assert_eq!(doc.history(), history);
    assert!(doc.proposal_info(id).is_ok(), "a stale verification leaves the proposal open");

    let fresh = doc.proposal_state(id).unwrap();
    let report2 = verify(&doc, id, &scripted(1));
    assert_ne!(report.candidate_start_checksum, report2.candidate_start_checksum);
    let accepted = doc.accept_if_unchanged(id, fresh).unwrap();
    assert_eq!(doc.checksum(), report2.candidate_start_checksum);
    assert!(accepted.history_id.is_some());
    doc.undo().unwrap();
    assert_eq!(state(&doc), unchanged, "the guarded acceptance is one undo entry");
}

#[test]
fn verified_accept_refuses_proposal_mutation_and_cross_proposal_state() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 12));
    let expected = doc.proposal_state(id).unwrap();
    let report = verify(&doc, id, &scripted(1));
    let body = guid_named(&doc, "body_05");
    doc.proposal_apply(id, set(&body, BODY, "pos", vec2(0, -30))).unwrap();
    let unchanged = state(&doc);
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));
    assert_eq!(state(&doc), unchanged);
    assert_ne!(report.candidate_start_checksum, doc.proposal_preview(id).unwrap().checksum());
    assert!(doc.history().is_empty());

    let other = propose_move(&mut doc, "body_05", vec2(0, 12));
    assert_eq!(doc.proposal_state(other).unwrap().proposal_revision, expected.proposal_revision);
    assert_eq!(doc.accept_if_unchanged(other, expected), Err(EditError::StaleVerification { proposal: other.0 }));
    assert!(doc.proposal_info(other).is_ok());
}

#[test]
fn verified_accept_invalidates_after_undo_redo_and_rollback_even_when_values_return() {
    let mut doc = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 12));
    let other = guid_named(&doc, "body_06");
    let expected = doc.proposal_state(id).unwrap();
    let initial = state(&doc);
    doc.apply(set(&other, BODY, "pos", vec2(2, 14)), Origin::User).unwrap();
    doc.undo().unwrap();
    assert_eq!(state(&doc), initial);
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));

    let expected = doc.proposal_state(id).unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));
    let expected = doc.proposal_state(id).unwrap();
    let before = state(&doc);
    doc.begin_tx("temporary", Origin::User).unwrap();
    doc.apply(set(&other, BODY, "pos", vec2(3, 15)), Origin::User).unwrap();
    doc.rollback_tx().unwrap();
    assert_eq!(state(&doc), before);
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));
}

#[test]
fn verified_accept_preserves_noops_but_conservatively_invalidates_failed_batch_rollback() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_05");
    let id = propose_move(&mut doc, "body_05", vec2(0, 12));
    let expected = doc.proposal_state(id).unwrap();
    assert!(!doc.proposal_apply(id, set(&body, BODY, "pos", vec2(0, 12))).unwrap().changed);
    assert_eq!(doc.proposal_state(id).unwrap(), expected);
    doc.save_yaml();
    assert_eq!(doc.proposal_state(id).unwrap(), expected, "saving does not change verification inputs");
    // An invalid first op rolls an empty transaction back. This deliberately
    // invalidates the staged revision even though no values changed.
    let preview = doc.proposal_preview(id).unwrap().checksum();
    assert!(doc.proposal_apply_all(id, vec![set(&body, BODY, "no_such_field", fixed(1))]).is_err());
    assert_eq!(doc.proposal_preview(id).unwrap().checksum(), preview);
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));
    let fresh = doc.proposal_state(id).unwrap();
    verify(&doc, id, &scripted(1));
    doc.accept_if_unchanged(id, fresh).unwrap();
}

#[test]
fn manual_accept_keeps_its_explicit_rebase_semantics() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_05");
    let id = propose_move(&mut doc, "body_05", vec2(0, 12));
    let expected = doc.proposal_state(id).unwrap();
    verify(&doc, id, &scripted(1));
    doc.apply(set(&body, BODY, "pos", vec2(3, 15)), Origin::User).unwrap();
    assert_eq!(doc.accept_if_unchanged(id, expected), Err(EditError::StaleVerification { proposal: id.0 }));
    doc.accept(id).unwrap();
    assert_eq!(doc.view().field(&orr_edit::Target::Guid(body), BODY, "pos").unwrap(), vec2(0, 12));
}

#[test]
fn verification_states_cannot_be_reused_on_another_document_with_matching_counters() {
    let mut doc = demo_doc();
    let mut other = demo_doc();
    let id = propose_move(&mut doc, "body_05", vec2(0, 12));
    let other_id = propose_move(&mut other, "body_05", vec2(0, -30));
    let expected = doc.proposal_state(id).unwrap();
    let other_state = other.proposal_state(other_id).unwrap();
    assert_eq!(expected.id, other_state.id);
    assert_eq!(expected.document_revision, other_state.document_revision);
    assert_eq!(expected.proposal_revision, other_state.proposal_revision);
    assert_ne!(expected.document_id, other_state.document_id);
    let before = state(&other);
    assert_eq!(other.accept_if_unchanged(other_id, expected), Err(EditError::StaleVerification { proposal: other_id.0 }));
    assert_eq!(state(&other), before);
    assert!(other.history().is_empty());
    assert!(other.proposal_info(other_id).is_ok());
}
