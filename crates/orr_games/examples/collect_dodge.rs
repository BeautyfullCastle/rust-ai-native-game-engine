//! Deterministic headless playable-loop oracle, not a graphical game host.
use orr_fp::{FPVec2, FP};
use orr_games::collect_dodge_game::{
    CollectDodgeV1, CollectInput, CollectLevel, CollectRun, HazardSpec, LOST_HAZARD, RESTART, WON,
};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
fn point(x: i64, y: i64) -> FPVec2 {
    FPVec2::new(FP(x << 16), FP(y << 16))
}
fn main() {
    let level = CollectLevel::new(
        point(0, 0),
        vec![point(10, 0), point(20, 0)],
        vec![HazardSpec {
            position: point(0, 12),
            velocity: point(0, 0),
        }],
        600,
    )
    .expect("bounded built-in level");
    let mut game = Simulation::<CollectDodgeV1>::new(level, 60, 42);
    // First collect both coins; then restart the same level and walk into danger.
    for tick in 1..=25 {
        let mut inputs = TickInputs::new(tick, 1);
        let input = if tick <= 16 {
            CollectInput {
                x: FP::ONE,
                ..Default::default()
            }
        } else if tick == 17 {
            CollectInput {
                buttons: RESTART,
                ..Default::default()
            }
        } else {
            CollectInput {
                y: FP::ONE,
                ..Default::default()
            }
        };
        inputs.set_input(PlayerSlot(0), input);
        for event in game.step(&inputs) {
            println!("tick={tick} event={:?}", event.payload);
        }
        if tick == 16 {
            assert_eq!(game.frame().singleton::<CollectRun>().phase, WON);
        }
    }
    assert_eq!(game.frame().singleton::<CollectRun>().phase, LOST_HAZARD);
    println!(
        "win -> restart -> hazard loss verified; checksum={:016x}",
        game.checksum()
    );
}
