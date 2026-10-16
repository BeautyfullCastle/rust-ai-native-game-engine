mod common;

use common::demo_doc;
use orr_edit::{EditError, PlayController, VerifyInputs, VerifyOptions};
use orr_sample::physics_game::{PhysGame, PhysInput, PhysMetrics};
use orr_session::ControlOp;
use orr_sim::PlayerSlot;

fn script(tick: u64, slot: PlayerSlot) -> PhysInput {
    PhysInput::new(if slot.0 == 0 { 1 } else { -1 }, (tick % 3) as i32 - 1, 0, tick % 7 == 0)
}

#[test]
fn capped_recording_matches_scripted_prefix_and_sampling() {
    let doc = demo_doc();
    let mut play = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    for tick in 1..=64 {
        for slot in 0..2 {
            play.session_mut().set_input(PlayerSlot(slot), script(tick, PlayerSlot(slot)));
        }
        play.control(ControlOp::Step(1));
    }
    let stopped = play.stop_play();
    let recorded = VerifyInputs::<PhysGame>::from_stopped(&stopped).unwrap();
    for cap in [15, 16] {
        for parallel in [false, true] {
            let opts = VerifyOptions { sample_every: 8, max_ticks: Some(cap), parallel, ..VerifyOptions::default() };
            let mut actual = doc.verify_self(&recorded, &PhysMetrics, &opts).unwrap();
            let expected = doc.verify_self(&VerifyInputs::<PhysGame>::scripted(cap, 2, script), &PhysMetrics, &opts).unwrap();
            let recording = actual.recording.take().unwrap();
            assert_eq!(recording.checked, cap + 1); // Includes the initial tick-zero checksum.
            assert_eq!(recording.mismatches, 0);
            assert_eq!(recording.first_mismatch, None);
            assert_eq!(actual.samples.iter().map(|s| s.tick).collect::<Vec<_>>(), vec![0, 8, u64::from(cap)]);
            assert!(!actual.metrics.is_empty());
            let capped_script = doc.verify_self(&VerifyInputs::<PhysGame>::scripted(64, 2, script), &PhysMetrics, &opts).unwrap();
            assert_eq!(capped_script, expected);
            assert_eq!(actual, expected);
        }
    }
    let zero = VerifyOptions { max_ticks: Some(0), ..VerifyOptions::default() };
    assert!(matches!(doc.verify_self(&recorded, &PhysMetrics, &zero), Err(EditError::Verify(m)) if m == "no ticks to run"));
    let mut later = orr_ecs::Frame::from_bytes(doc.frame_registry().clone(), &doc.frame().to_bytes()).unwrap();
    later.set_tick(1);
    assert!(
        matches!(orr_edit::verify_frames(&later, &later, &recorded, &PhysMetrics, &zero), Err(EditError::Verify(m)) if m.contains("starts at tick 1"))
    );
}

#[test]
fn recorded_empty_and_contiguous_prefix_rules_survive_caps() {
    let doc = demo_doc();
    let play = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    let stopped = play.stop_play();
    let empty = orr_session::ReplayReader::<PhysGame>::parse(&stopped.replay).unwrap();
    let mut writer = orr_session::ReplayWriter::<PhysGame>::new(empty.header.clone());
    for tick in [1, 2, 4] {
        writer.record_tick(tick, &[script(tick, PlayerSlot(0)), script(tick, PlayerSlot(1))], &[]);
    }
    let gap = VerifyInputs::<PhysGame>::from_replay(&writer.finish()).unwrap();
    for cap in [None, Some(0), Some(1), Some(2), Some(3)] {
        let opts = VerifyOptions { max_ticks: cap, parallel: false, ..VerifyOptions::default() };
        assert!(
            matches!(doc.verify_self(&VerifyInputs::Recorded(orr_session::ReplayReader::<PhysGame>::parse(&stopped.replay).unwrap()), &PhysMetrics, &opts), Err(EditError::Verify(m)) if m == "the recording has no ticks")
        );
        if cap == Some(0) {
            assert!(matches!(doc.verify_self(&gap, &PhysMetrics, &opts), Err(EditError::Verify(m)) if m == "no ticks to run"));
        } else {
            let mut actual = doc.verify_self(&gap, &PhysMetrics, &opts).unwrap();
            let ticks = cap.unwrap_or(2).min(2);
            actual.recording = None;
            let expected = doc.verify_self(&VerifyInputs::<PhysGame>::scripted(ticks, 2, script), &PhysMetrics, &opts).unwrap();
            assert_eq!(actual, expected);
        }
    }
}
