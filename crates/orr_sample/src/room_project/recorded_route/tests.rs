use super::*;
use orr_session::{ReplayHeader, ReplayWriter};
use orr_sim::Simulation;
fn scene() -> PreparedScene {
    let sim = Simulation::<game::RoomEscapeV1>::new(game::RoomConfig, game::TICK_RATE, SEED);
    let text = orr_reflect::Scene::unbake(&super::super::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    PreparedScene::parse(&text).unwrap()
}
fn recording(
    scene: &PreparedScene,
    count: u64,
    alternate: bool,
) -> (ReplayWriter<game::RoomEscapeV1>, u64) {
    let mut sim = scene.simulation().unwrap();
    let mut writer = ReplayWriter::new(ReplayHeader {
        format_version: 3,
        game_id: GAME.into(),
        build_hash: sim.build_hash(),
        seed: SEED,
        player_count: 1,
        tick_rate: 60,
        input_size: 8,
    });
    writer.record_keyframe(sim.frame());
    writer.record_checksum(0, sim.frame().checksum());
    for tick in 1..=count {
        let input = game::RoomInput {
            move_x: if alternate {
                (tick % 2) as i8
            } else if tick > 1 && tick < count {
                1
            } else {
                0
            },
            buttons: if tick == 1 || tick == count { 1 } else { 0 },
            ..Default::default()
        };
        writer.record_tick(tick, &[input], &[]);
        let mut inputs = TickInputs::new(tick, 1);
        inputs.set_input(PlayerSlot(0), input);
        sim.step(&inputs);
        writer.record_checksum(tick, sim.frame().checksum());
    }
    (writer, sim.frame().checksum())
}
#[test]
fn winning_route_replays_and_preserves_release() {
    let scene = scene();
    let (w, checksum) = recording(&scene, 62, false);
    let script = convert(&scene, &w.to_bytes(), 62, checksum).unwrap();
    let v: serde_json::Value = serde_json::from_str(&script).unwrap();
    let steps = v["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0]["expect_key"], true);
    assert_eq!(steps[1]["interact"], false);
    assert_eq!(steps[2]["expect_won"], true);
    assert_eq!(
        script,
        convert(&scene, &w.to_bytes(), 62, checksum).unwrap()
    );
}
#[test]
fn rejects_debug_commands_at_any_tick_and_simulation_commands() {
    let scene = scene();
    for tick in [0, 1, 63, u64::MAX] {
        let (mut w, checksum) = recording(&scene, 62, false);
        w.record_debug(
            tick,
            orr_sim::DebugCommand::Spawn {
                components: Vec::new(),
            },
        );
        assert!(ReplayReader::<game::RoomEscapeV1>::parse(&w.to_bytes())
            .unwrap()
            .has_debug_commands());
        assert!(convert(&scene, &w.to_bytes(), 62, checksum)
            .unwrap_err()
            .contains("debug"));
    }
    let (mut w, checksum) = recording(&scene, 62, false);
    w.record_tick(
        1,
        &[game::RoomInput::default()],
        &[(PlayerSlot(0), game::NoCommand::default())],
    );
    assert!(convert(&scene, &w.to_bytes(), 62, checksum)
        .unwrap_err()
        .contains("commands"));
}
#[test]
fn rejects_invalid_identity_corruption_missing_ticks_and_stop_receipts() {
    let scene = scene();
    let (w, checksum) = recording(&scene, 62, false);
    let bytes = w.to_bytes();
    for (tick, sum) in [
        (0, checksum),
        (61, checksum),
        (6001, checksum),
        (62, checksum ^ 1),
    ] {
        assert!(convert(&scene, &bytes, tick, sum).is_err());
    }
    let mut changed = bytes.clone();
    changed[12] ^= 1;
    assert!(convert(&scene, &changed, 62, checksum).is_err());
    let mut wrong = bytes.clone();
    wrong[12 + GAME.len()] ^= 1;
    assert!(convert(&scene, &wrong, 62, checksum).is_err());
    for length in [0, 3, 12, bytes.len() - 1] {
        assert!(convert(&scene, &bytes[..length], 62, checksum).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(convert(&scene, &extra, 62, checksum).is_err());
    let (mut w, checksum) = recording(&scene, 62, false);
    w.record_tick(
        1,
        &[game::RoomInput {
            reserved: 1,
            ..Default::default()
        }],
        &[],
    );
    assert!(convert(&scene, &w.to_bytes(), 62, checksum)
        .unwrap_err()
        .contains("malformed"));
    let (mut w, checksum) = recording(&scene, 62, false);
    w.record_checksum(0, 0);
    assert!(convert(&scene, &w.to_bytes(), 62, checksum).is_err());
    let (mut w, checksum) = recording(&scene, 62, false);
    w.record_tick(64, &[game::RoomInput::default()], &[]);
    assert!(convert(&scene, &w.to_bytes(), 64, checksum).is_err());
}
#[test]
fn bounds_steps_ticks_and_preparse_memory() {
    let scene = scene();
    for count in [256, 257] {
        let (w, sum) = recording(&scene, count, true);
        assert_eq!(
            convert(&scene, &w.to_bytes(), count, sum).is_ok(),
            count == 256
        );
    }
    let (w, sum) = recording(&scene, 6000, false);
    assert!(convert(&scene, &w.to_bytes(), 6000, sum).is_ok());
    let mut maximum_length = w.to_bytes();
    let length_offset = 12 + GAME.len() + 8 + 8 + 1 + 4 + 4;
    maximum_length[length_offset..length_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(convert(&scene, &maximum_length, 6000, sum)
        .unwrap_err()
        .contains("size"));
    let mut b = w.to_bytes();
    let offset = 12 + GAME.len() + 8 + 8 + 1 + 4 + 4 + 4;
    b[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(convert(&scene, &b, 6000, sum)
        .unwrap_err()
        .contains("decompression"));
    assert!(convert(&scene, &vec![0; MAX_REPLAY_BYTES + 1], 6000, sum).is_err());
}

#[test]
fn rejects_nonzero_start_missing_initial_frame_wrong_frame_and_intermediate_checksum() {
    let scene = scene();
    let (original, checksum) = recording(&scene, 62, false);
    let reader = ReplayReader::<game::RoomEscapeV1>::parse(&original.to_bytes()).unwrap();
    for case in 0..4 {
        let mut writer = ReplayWriter::<game::RoomEscapeV1>::new(reader.header.clone());
        if case != 1 {
            let mut initial = scene.frame().clone();
            if case == 2 {
                initial.set_singleton(orr_fp::FrameRng::new(SEED + 1));
            }
            writer.record_keyframe(&initial);
        }
        for tick in 1..=62 {
            if case == 0 && tick == 1 {
                continue;
            }
            let (inputs, commands) = reader.tick(tick).unwrap();
            writer.record_tick(tick, inputs, commands);
        }
        for &(tick, sum) in &reader.checksums {
            writer.record_checksum(
                tick,
                if case == 3 && tick == 30 {
                    sum ^ 1
                } else {
                    sum
                },
            );
        }
        let error = convert(&scene, &writer.to_bytes(), 62, checksum).unwrap_err();
        if case == 3 {
            assert!(error.contains("tick 30"), "{error}");
        }
        if case == 1 || case == 2 {
            assert!(error.contains("initial Frame"), "{error}");
        }
    }
}
