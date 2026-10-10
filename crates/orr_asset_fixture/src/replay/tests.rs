use super::*;
use crate::game::{Motion, NoCommand, Position, TICK_CALLS};
use orr_ecs::Frame;
use orr_fp::FP;
use orr_sim::DebugCommand;

fn reset_ticks() {
    TICK_CALLS.with(|n| n.set(0));
}
fn ticks() -> u64 {
    TICK_CALLS.with(|n| n.get())
}
fn frames() -> Vec<Frame> {
    let mut sim = game::new_simulation();
    let mut frames = vec![sim.frame().clone()];
    for tick in 1..=TICKS {
        sim.step(&game::inputs(tick));
        frames.push(sim.frame().clone());
    }
    frames
}
fn writer(
    frames: &[Frame],
    tick_keys: &[u64],
    checksum_keys: &[u64],
    frame_keys: &[u64],
) -> ReplayWriter<AssetFixtureGame> {
    let mut writer = ReplayWriter::new(header());
    for &tick in tick_keys {
        writer.record_tick(
            tick,
            &[Input {
                axis: game::axis(tick),
            }],
            &[],
        );
    }
    for &tick in checksum_keys {
        writer.record_checksum(tick, frames[tick.min(TICKS) as usize].checksum());
    }
    for &tick in frame_keys {
        writer.record_keyframe(&frames[tick as usize]);
    }
    writer
}
fn normal(frames: &[Frame]) -> ReplayWriter<AssetFixtureGame> {
    let keys: Vec<u64> = (1..=TICKS).collect();
    writer(frames, &keys, &keys, &[60, 120, 180, 240, 300])
}
fn decode(writer: ReplayWriter<AssetFixtureGame>) -> ReplayReader<AssetFixtureGame> {
    ReplayReader::parse(&writer.finish()).unwrap()
}
fn rejected_without_steps(writer: ReplayWriter<AssetFixtureGame>) -> Error {
    let bytes = writer.finish();
    let prepared = PreparedFixture::embedded().unwrap();
    let reader = ReplayReader::parse(&bytes).unwrap(); // All variants use normal encoding.
    reset_ticks();
    assert!(matches!(
        prepared.verify(&bytes),
        Err(Error::UntrackedReplay)
    ));
    let error = validate_reader(&reader, BUILD_ID).expect_err("validator must reject");
    assert_eq!(ticks(), 0);
    error
}

#[test]
fn allowlist_budget_and_all_debug_forms_reject_before_any_resimulation() {
    let frames = frames();
    let entity = frames[0].entities().next().unwrap();
    let component = frames[0].registry().component_id::<Motion>().unwrap();
    let bad_ref = 0xdead_u64.to_ne_bytes().to_vec();
    let commands = [
        DebugCommand::SetField {
            entity,
            component,
            offset: 0,
            bytes: bad_ref.clone(),
        },
        DebugCommand::Spawn {
            components: vec![(component, bad_ref.clone())],
        },
        DebugCommand::AddComponent {
            entity,
            component,
            bytes: bad_ref,
        },
    ];
    let prepared = PreparedFixture::embedded().unwrap();
    for command in commands {
        for tick in [0, 1, 120, 300, 301] {
            let mut writer = normal(&frames);
            writer.record_debug(tick, command.clone());
            let bytes = writer.finish();
            let reader = ReplayReader::<AssetFixtureGame>::parse(&bytes).unwrap();
            assert_eq!(reader.debug_commands(tick), std::slice::from_ref(&command));
            reset_ticks();
            assert!(matches!(
                prepared.verify(&bytes),
                Err(Error::UntrackedReplay)
            ));
            assert!(matches!(
                prepared.seek(&bytes, 300),
                Err(Error::UntrackedReplay)
            ));
            if tick <= 300 {
                assert!(
                    matches!(validate_reader(&reader, BUILD_ID), Err(Error::DebugCommand(t)) if t == tick)
                );
            } else {
                // Public APIs cannot enumerate t301 debug. Only the closed byte
                // gate rejects it; never advertise validate_reader as sufficient.
                validate_reader(&reader, BUILD_ID).unwrap();
            }
            assert_eq!(ticks(), 0);
        }
    }
    reset_ticks();
    assert!(matches!(
        prepared.verify(&vec![0; MAX_INPUT_BYTES + 1]),
        Err(Error::BudgetExceeded)
    ));
    assert!(matches!(
        prepared.verify(b"not a known recording"),
        Err(Error::UntrackedReplay)
    ));
    assert_eq!(ticks(), 0);
}

#[test]
fn header_validator_directly_rejects_each_mismatch_including_zero_expected_id() {
    let bytes = include_bytes!("../../fixtures/baseline.orrp");
    let reader = ReplayReader::<AssetFixtureGame>::parse(bytes).unwrap();
    reset_ticks();
    validate_header(&reader.header, BUILD_ID).unwrap();
    assert!(validate_header(&reader.header, 0).is_err());
    for mutate in [
        |h: &mut ReplayHeader| h.build_hash = 0,
        |h: &mut ReplayHeader| h.build_hash ^= 1,
        |h: &mut ReplayHeader| h.build_hash = orr_sim::build_hash_of(BUILD_ID, 1),
        |h: &mut ReplayHeader| h.game_id = "arena".into(),
        |h: &mut ReplayHeader| h.format_version = 2,
        |h: &mut ReplayHeader| h.seed = 2,
        |h: &mut ReplayHeader| h.player_count = 0,
        |h: &mut ReplayHeader| h.player_count = 2,
        |h: &mut ReplayHeader| h.tick_rate = 30,
        |h: &mut ReplayHeader| h.input_size = 8,
    ] {
        let mut header = reader.header.clone();
        mutate(&mut header);
        assert!(matches!(
            validate_header(&header, BUILD_ID),
            Err(Error::Header(_))
        ));
    }
    assert_eq!(ticks(), 0);
}

#[test]
fn tick_validator_rejects_sparse_missing_extra_zero_and_wrong_schedules() {
    let frames = frames();
    let keys: Vec<u64> = (1..=TICKS).collect();
    for tick_keys in [
        (1..300).collect::<Vec<_>>(),
        (2..=300).collect(),
        (1..=300).filter(|&t| t != 150).collect(),
        (0..=300).collect(),
        (1..=301).collect(),
        Vec::new(),
    ] {
        assert!(matches!(
            rejected_without_steps(writer(
                &frames,
                &tick_keys,
                &keys,
                &[60, 120, 180, 240, 300]
            )),
            Error::Ticks
        ));
    }
    for (tick, bad_axis) in [(1, 0), (120, -1), (121, 1), (180, -1), (181, 0), (300, 2)] {
        let mut writer = normal(&frames);
        writer.record_tick(tick, &[Input { axis: bad_axis }], &[]);
        assert!(matches!(rejected_without_steps(writer), Error::Input(t) if t == tick));
    }
    let mut writer = normal(&frames);
    writer.record_tick(
        150,
        &[Input { axis: 0 }],
        &[(orr_sim::PlayerSlot(0), NoCommand)],
    );
    assert!(matches!(rejected_without_steps(writer), Error::Input(150)));

    // Correctly encoded two-player inputs are checked independently of header.
    let mut two_header = header();
    two_header.player_count = 2;
    let mut two = ReplayWriter::<AssetFixtureGame>::new(two_header);
    for tick in 1..=TICKS {
        two.record_tick(
            tick,
            &[Input {
                axis: game::axis(tick),
            }; 2],
            &[],
        );
    }
    let two = decode(two);
    reset_ticks();
    assert!(matches!(validate_ticks(&two), Err(Error::Input(1))));
    assert_eq!(ticks(), 0);
}

#[test]
fn checksum_validator_requires_exact_ordered_unique_complete_coverage() {
    let frames = frames();
    let keys: Vec<u64> = (1..=TICKS).collect();
    let mut duplicate = keys.clone();
    duplicate[149] = 149;
    let mut reversed = keys.clone();
    reversed.reverse();
    let mut extra = keys.clone();
    extra.push(300);
    for checksums in [
        Vec::new(),
        (1..300).collect(),
        duplicate,
        reversed,
        extra,
        (0..300).collect(),
        (2..=301).collect(),
    ] {
        assert!(matches!(
            rejected_without_steps(writer(
                &frames,
                &keys,
                &checksums,
                &[60, 120, 180, 240, 300]
            )),
            Error::Checksums
        ));
    }
    let reader = decode(normal(&frames));
    validate_checksums(&reader).unwrap();
    assert_eq!(reader.checksums.len(), 300);
    assert_eq!(
        reader
            .checksums
            .iter()
            .map(|&(tick, _)| tick)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        300
    );
    assert_eq!(
        reader
            .checksums
            .iter()
            .map(|&(_, checksum)| checksum)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        300
    );
}

#[test]
fn exact_key_preflight_checks_all_keys_without_resimulation_and_rejects_key_sets() {
    let frames = frames();
    let reader = decode(normal(&frames));
    reset_ticks();
    validate_reader(&reader, BUILD_ID).unwrap();
    assert_eq!(ticks(), 0);
    for &key in &KEYS_DESCENDING {
        assert_eq!(reader.nearest_keyframe(key), Some(key));
        let sim = reader.seek((), key).unwrap();
        assert_eq!(sim.build_id(), 0); // This must never escape as a branch source.
        assert_eq!(sim.checksum(), frames[key as usize].checksum());
    }
    assert_eq!(ticks(), 0);
    let keys: Vec<u64> = (1..=TICKS).collect();
    for keyframes in [
        vec![],
        vec![60, 120, 180, 240],
        vec![0, 60, 120, 180, 240, 300],
        vec![59, 120, 180, 240, 300],
        (0..=64).collect(),
    ] {
        assert!(matches!(
            rejected_without_steps(writer(&frames, &keys, &keys, &keyframes)),
            Error::Keyframes
        ));
    }
    let mut future = frames[300].clone();
    future.set_tick(301);
    let mut writer = normal(&frames);
    writer.record_keyframe(&future);
    assert!(matches!(rejected_without_steps(writer), Error::Keyframes));
}

#[test]
fn exact_key_preflight_rejects_refs_shape_range_and_checksum_before_steps() {
    let frames = frames();
    let entity = frames[60].entities().next().unwrap();
    for mutate in [
        |frame: &mut Frame| {
            let entity = frame.entities().next().unwrap();
            frame.get_mut::<Motion>(entity).unwrap().profile = crate::MOTION_ID;
            frame.remove::<Motion>(entity);
        },
        |frame: &mut Frame| {
            let entity = frame.entities().next().unwrap();
            frame.get_mut::<Motion>(entity).unwrap().profile = orr_asset::AssetRef::NULL;
        },
        |frame: &mut Frame| {
            let entity = frame.entities().next().unwrap();
            frame.get_mut::<Motion>(entity).unwrap().profile =
                orr_asset::AssetRef::from_raw(0xdead);
        },
        |frame: &mut Frame| {
            let entity = frame.entities().next().unwrap();
            frame.remove::<Position>(entity);
        },
        |frame: &mut Frame| {
            frame.spawn();
        },
        |frame: &mut Frame| {
            let entity = frame.entities().next().unwrap();
            frame.get_mut::<Position>(entity).unwrap().x = FP::from_raw(-1);
        },
        |frame: &mut Frame| {
            let entity = frame.entities().next().unwrap();
            frame.get_mut::<Position>(entity).unwrap().x = FP::from_raw(491521);
        },
    ] {
        let mut bad = frames[60].clone();
        mutate(&mut bad);
        let mut writer = normal(&frames);
        writer.record_keyframe(&bad);
        rejected_without_steps(writer);
    }
    let mut altered = frames[60].clone();
    altered.get_mut::<Position>(entity).unwrap().x = FP::ZERO;
    let mut writer = normal(&frames);
    writer.record_keyframe(&altered);
    assert!(matches!(
        rejected_without_steps(writer),
        Error::Checksum(60)
    ));
}

#[test]
fn result_validators_reject_partial_verification_and_inexact_seek() {
    reset_ticks();
    validate_report(&VerifyReport {
        ticks_simulated: 300,
        checksums_checked: 300,
        mismatch: None,
    })
    .unwrap();
    for report in [
        VerifyReport {
            ticks_simulated: 299,
            checksums_checked: 300,
            mismatch: None,
        },
        VerifyReport {
            ticks_simulated: 301,
            checksums_checked: 300,
            mismatch: None,
        },
        VerifyReport {
            ticks_simulated: 300,
            checksums_checked: 0,
            mismatch: None,
        },
        VerifyReport {
            ticks_simulated: 300,
            checksums_checked: 299,
            mismatch: None,
        },
        VerifyReport {
            ticks_simulated: 300,
            checksums_checked: 301,
            mismatch: None,
        },
        VerifyReport {
            ticks_simulated: 300,
            checksums_checked: 300,
            mismatch: Some((1, 2, 3)),
        },
    ] {
        assert!(matches!(validate_report(&report), Err(Error::Verification)));
    }
    assert_eq!(ticks(), 0);
    let frames = frames();
    reset_ticks();
    for tick in [0, 60, 120, 180, 240, 300] {
        let frame = &frames[tick as usize];
        validate_seek_result(frame, tick, frame.checksum()).unwrap();
        assert!(
            matches!(validate_seek_result(frame, tick, frame.checksum() ^ 1), Err(Error::Checksum(t)) if t == tick)
        );
        assert!(validate_seek_result(frame, tick + 1, frame.checksum()).is_err());
    }
    assert_eq!(ticks(), 0);
}

#[test]
fn keyframe_with_incompatible_registry_fails_decode_before_resimulation() {
    let frames = frames();
    let mut wrong = Frame::new(orr_ecs::ComponentRegistryBuilder::new().build());
    wrong.set_tick(60);
    let mut writer = normal(&frames);
    writer.record_keyframe(&wrong);
    assert!(matches!(
        rejected_without_steps(writer),
        Error::Replay(orr_session::ReplayError::BadKeyframe { tick: 60, .. })
    ));
}
