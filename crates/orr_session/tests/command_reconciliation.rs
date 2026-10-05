//! Command-only prediction differences must reconcile without wall-clock timing.
use orr_session::{AdvanceResult, InputSource, LoopbackNetwork, Session, SessionConfig};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

#[test]
fn late_commands_with_matching_inputs_converge_to_headless() {
    delayed_matching_inputs(true);
}

#[test]
fn late_empty_commands_with_matching_inputs_do_not_roll_back() {
    delayed_matching_inputs(false);
}

fn delayed_matching_inputs(with_commands: bool) {
    let (a, b, clock) = LoopbackNetwork::new::<Arena>(3, 0, 42);
    let config = |slot| {
        let mut cfg = SessionConfig::new(2, PlayerSlot(slot), 42, 60);
        cfg.input_delay = 0;
        cfg.checksum_interval = 1;
        cfg
    };
    let mut peers = [
        Session::new(ArenaConfig { player_count: 2 }, config(0), a),
        Session::new(ArenaConfig { player_count: 2 }, config(1), b),
    ];
    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 42);
    let mut expected = Vec::new();
    for tick in 1..=3 {
        clock.tick();
        let mut inputs = TickInputs::new(tick, 2);
        let mut commands = Vec::new();
        for (slot, peer) in peers.iter_mut().enumerate() {
            // The input is exactly the default prediction. Commands are independent
            // of its FIRE bit and arrive only after all three ticks were simulated.
            let own = if with_commands && tick == 1 {
                vec![SpawnBulletCmd { owner: slot as u32 }]
            } else {
                Vec::new()
            };
            commands.extend(own.iter().cloned().map(|cmd| (PlayerSlot(slot as u8), cmd)));
            assert!(matches!(
                peer.advance(ArenaInput::default(), own),
                AdvanceResult::Advanced { .. }
            ));
        }
        inputs.set_commands(commands);
        reference.step(&inputs);
        expected.push((tick, reference.checksum()));
    }
    for peer in &peers {
        assert_eq!(peer.verified_tick(), 0);
        assert_eq!(peer.rollback_count(), 0);
    }
    // Deliver every original packet, without advancing either simulation.
    for _ in 0..3 {
        clock.tick();
    }
    for peer in &mut peers {
        let (_, rollback) = peer.poll_confirmed();
        assert_eq!(peer.verified_tick(), 3);
        assert_eq!(
            peer.checksums(),
            expected,
            "verified command history must match headless"
        );
        if with_commands {
            let rollback = rollback.expect("a commands-only difference must roll back");
            assert_eq!(
                (rollback.from_tick, rollback.to_tick, rollback.resim_count),
                (1, 3, 3)
            );
        } else {
            assert!(
                rollback.is_none(),
                "matching inputs without commands need no rollback"
            );
        }
        assert!(peer.poll_confirmed().1.is_none());
    }
    assert_eq!(peers[0].checksums(), peers[1].checksums());

    // A later conflicting packet must not rewrite already-verified history.
    peers[0].source_mut().send_local(
        1,
        PlayerSlot(0),
        ArenaInput::default(),
        vec![SpawnBulletCmd { owner: 1 }, SpawnBulletCmd { owner: 1 }],
    );
    for _ in 0..3 {
        clock.tick();
    }
    let (events, rollback) = peers[1].poll_confirmed();
    assert!(events.is_empty());
    assert!(rollback.is_none());
    assert_eq!(peers[1].verified_tick(), 3);
    assert_eq!(peers[1].checksums(), expected);
}
