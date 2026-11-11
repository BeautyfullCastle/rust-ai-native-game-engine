mod common;
use common::*;
use orr_fp::FP;
use orr_terrain::Terrain;
use orr_terrain_demo::{
    controller::{CARDINAL_STEP_RAW, DIAGONAL_STEP_RAW},
    ControlKey, Controls, Course, FixedClock, Game, Input,
};
use std::time::Duration;

#[test]
fn default_course_completes_through_real_keyboard_ticks_and_restart_matches() {
    let mut g = Game::new(Course::builtin().unwrap());
    let initial = g.state();
    let terrain = g.course().terrain().cook();
    let mut counts = Vec::new();
    for (input, ticks) in builtin_script() {
        for _ in 0..ticks {
            g.tick(input);
            let s = g.state();
            assert_eq!(
                g.course().terrain().sample(s.anchor[0], s.anchor[2]),
                Some(s.anchor[1])
            );
        }
        counts.push(g.state().collected);
    }
    assert_eq!(counts, [0, 1, 1, 2, 2, 3, 3, 4, 4, 4]);
    assert!(g.state().finished);
    assert!(g.state().tick <= 448);
    let won = g.state();
    g.tick(Input { x: 1, z: 1 });
    assert_eq!(g.state(), won);
    assert_eq!(g.course().terrain().cook(), terrain);
    g.restart();
    assert_eq!(g.state(), initial);
}

#[test]
fn identical_inputs_produce_identical_supported_states() {
    let mut a = Game::new(Course::builtin().unwrap());
    let mut b = Game::new(Course::builtin().unwrap());
    for tick in 0..1000 {
        let input = Input {
            x: (tick % 3) as i8 - 1,
            z: ((tick / 7) % 3) as i8 - 1,
        };
        a.tick(input);
        b.tick(input);
        assert_eq!(a.state(), b.state());
    }
}

#[test]
fn cardinal_and_integer_diagonal_speeds_are_bounded() {
    let mut g = Game::new(simple(["0", "0"]));
    g.tick(Input { x: 127, z: 0 });
    assert_eq!(g.state().anchor[0].raw(), CARDINAL_STEP_RAW);
    g.restart();
    g.tick(Input { x: -128, z: 127 });
    assert_eq!(
        [g.state().anchor[0].raw(), g.state().anchor[2].raw()],
        [-DIAGONAL_STEP_RAW, DIAGONAL_STEP_RAW]
    );
    const {
        assert!(2 * DIAGONAL_STEP_RAW * DIAGONAL_STEP_RAW <= CARDINAL_STEP_RAW * CARDINAL_STEP_RAW);
    }
}

#[test]
fn opposing_and_duplicate_bindings_and_focus_clear_are_exact() {
    let mut c = Controls::default();
    c.set(ControlKey::W, true);
    c.set(ControlKey::Up, true);
    c.set(ControlKey::W, false);
    assert_eq!(c.input(), Input { x: 0, z: -1 });
    c.set(ControlKey::S, true);
    assert_eq!(c.input(), Input::default());
    c.set(ControlKey::D, true);
    c.set(ControlKey::Left, true);
    assert_eq!(c.input(), Input::default());
    c.set(ControlKey::Right, true);
    c.set(ControlKey::A, true);
    c.set(ControlKey::Down, true);
    c.clear();
    assert_eq!(c.input(), Input::default());
}

#[test]
fn exact_owner_seams_and_negative_hole_corners_block_without_diagonal_jump() {
    // Hole [-0.75,-0.5) x [-0.75,-0.5), smallest admitted spacing.
    let mut holes = vec![false; 16];
    holes[5] = true;
    let t = terrain(5, 5, [-1, -1], FP::from_raw(16384), holes);
    let make = |spawn| course(&t, spawn, [["0", "0"]; 4], ["0", "0"]);
    let mut g = Game::new(make(["-0.78125", "-0.765625"]));
    g.tick(Input { x: 1, z: 1 });
    // X may cross the corner while still south of the hole; Z then hits its owner.
    assert_eq!(g.state().anchor[0].raw(), -51200 + DIAGONAL_STEP_RAW);
    assert_eq!(g.state().anchor[2].raw(), -50176);
    assert_eq!(
        t.sample(g.state().anchor[0], g.state().anchor[2]),
        Some(g.state().anchor[1])
    );
    let mut g = Game::new(make(["-0.8125", "-0.625"]));
    g.tick(Input { x: 1, z: 0 });
    assert_eq!(g.state().anchor[0], FP::parse("-0.8125").unwrap()); // exact low seam owns hole
    let mut g = Game::new(make(["-0.5", "-0.625"]));
    g.tick(Input { x: -1, z: 0 });
    assert_eq!(g.state().anchor[0], FP::parse("-0.5").unwrap()); // high seam belongs to next solid cell
}

#[test]
fn hole_barrier_cannot_be_crossed_by_repeated_positive_or_negative_steps() {
    let mut holes = vec![false; 16];
    for row in 0..4 {
        holes[row * 4 + 1] = true;
    }
    let t = terrain(5, 5, [-1, -1], FP::from_raw(16384), holes);
    for (spawn, input) in [
        (["-1", "-0.5"], Input { x: 1, z: 0 }),
        (["0", "-0.5"], Input { x: -1, z: 0 }),
    ] {
        let mut g = Game::new(course(&t, spawn, [["0", "0"]; 4], ["0", "0"]));
        for _ in 0..100 {
            g.tick(input);
            assert!(t.sample(g.state().anchor[0], g.state().anchor[2]).is_some());
        }
        assert!(if input.x > 0 {
            g.state().anchor[0].raw() < -49152
        } else {
            g.state().anchor[0].raw() >= -32768
        });
    }
}

#[test]
fn outer_maxima_and_negative_origin_edges_never_leave_surface() {
    for (spawn, input) in [
        (["-8", "-8"], Input { x: -1, z: -1 }),
        (["8", "8"], Input { x: 1, z: 1 }),
    ] {
        let mut g = Game::new(simple(spawn));
        let anchor = g.state().anchor;
        for _ in 0..100 {
            g.tick(input);
        }
        assert_eq!(g.state().anchor, anchor);
    }
}

#[test]
fn sampled_y_uses_canonical_triangles_and_negative_height_floor() {
    let t = Terrain::new(
        "triangle".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::from_int(1),
        [0, 4, 2, 10].map(FP::from_int).to_vec(),
        vec![false],
    )
    .unwrap();
    let make = |spawn| course(&t, spawn, [["1", "1"]; 4], ["1", "1"]);
    let mut g = Game::new(make(["0.25", "0.5"]));
    g.tick(Input { x: 1, z: 0 });
    assert_eq!(g.state().anchor[1], FP::parse("3.5").unwrap());
    let mut g = Game::new(make(["0.5", "0.25"]));
    g.tick(Input { x: 1, z: 0 });
    assert_eq!(g.state().anchor[1], FP::parse("3.75").unwrap());
    let t = Terrain::new(
        "floor".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::from_raw(20000),
        vec![FP::ZERO, FP::from_raw(-1), FP::ZERO, FP::from_raw(-1)],
        vec![false],
    )
    .unwrap();
    let mut g = Game::new(course(&t, ["0", "0"], [["0", "0"]; 4], ["0", "0"]));
    g.tick(Input { x: 1, z: 0 });
    assert_eq!(g.state().anchor[1].raw(), -1);
}

#[test]
fn ordered_beacons_never_skip_and_finish_requires_a_later_tick() {
    let t = flat();
    let mut g = Game::new(course(
        &t,
        ["0", "0"],
        [["1", "0"], ["0", "0"], ["1", "0"], ["0", "0"]],
        ["0", "0"],
    ));
    g.tick(Input::default());
    assert_eq!(g.state().collected, 0);
    for _ in 0..16 {
        g.tick(Input { x: 1, z: 0 });
    }
    assert_eq!(g.state().collected, 1);
    for _ in 0..16 {
        g.tick(Input { x: -1, z: 0 });
    }
    assert_eq!(g.state().collected, 2);
    let mut g = Game::new(course(&t, ["0", "0"], [["0", "0"]; 4], ["0", "0"]));
    for count in 1..=4 {
        g.tick(Input::default());
        assert_eq!(g.state().collected, count);
        assert!(!g.state().finished);
    }
    g.tick(Input::default());
    assert!(g.state().finished);
}

#[test]
fn rational_clock_frame_partitions_match_and_reset_discards_fraction() {
    let mut a = FixedClock::default();
    let mut b = FixedClock::default();
    let mut ta = 0;
    let mut tb = 0;
    for _ in 0..100 {
        ta += a.advance(Duration::from_millis(10));
    }
    for _ in 0..25 {
        tb += b.advance(Duration::from_millis(40));
    }
    assert_eq!((ta, tb), (60, 60));
    a.advance(Duration::from_millis(10));
    a.reset();
    assert_eq!(a.advance(Duration::from_millis(7)), 0);
}

#[test]
fn lag_cap_discards_whole_backlog_instead_of_fast_forwarding() {
    let mut c = FixedClock::default();
    assert_eq!(c.advance(Duration::from_secs(u64::MAX)), 8);
    assert_eq!(c.advance(Duration::ZERO), 0);
    assert_eq!(c.advance(Duration::from_millis(17)), 1);
    c.reset();
    assert_eq!(c.advance(Duration::from_nanos(16_666_666)), 0);
    assert_eq!(c.advance(Duration::from_nanos(1)), 1);
}
