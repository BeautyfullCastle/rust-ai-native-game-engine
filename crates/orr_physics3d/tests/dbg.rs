mod common;
use common::*;
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{PhysicsConfig, Scratch};

fn stack(n: i32, iters: u32, spacing: orr_fp::FP) {
    let mut cfg = PhysicsConfig::default();
    cfg.velocity_iterations = iters;
    cfg.sleep_ticks = 0;
    let mut f = new_frame_with(cfg);
    let mut sc = Scratch::new();
    ground(&mut f);
    let mut es = vec![];
    for i in 0..n {
        es.push(spawn_box(&mut f, v3!(0, 0.5, 0) + FPVec3::new(FP::ZERO, FP::from_int(i) * spacing, FP::ZERO), v3!(0.5, 0.5, 0.5)));
    }
    for t in 0..6 {
        run(&mut f, &mut sc, 50);
        let b = body(&f, es[(n - 1) as usize]);
        println!("n{} it{} t{} top y {:?} v {:?} w {:?} q {:?}", n, iters, t, b.pos.y, b.vel, b.omega, b.rot);
    }
}

#[test]
fn debug_stack_over_time() {
    stack(1, 8, fp!(1.0));
    stack(2, 8, fp!(1.0));
    stack(2, 8, fp!(1.01));
    stack(3, 8, fp!(1.0));
    stack(5, 8, fp!(1.0));
    stack(5, 20, fp!(1.0));
}
