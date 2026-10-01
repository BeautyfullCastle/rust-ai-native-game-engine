mod common;
use common::*;
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{PhysicsConfig, PhysicsState, Scratch};

fn stack(n: i32, cfg: PhysicsConfig) -> (FP, FP) {
    let mut f = new_frame();
    ground(&mut f);
    let mut es = vec![];
    for i in 0..n {
        let y = fp!(0.5) + FP::from_int(i) * fp!(1.002);
        es.push(spawn_box(&mut f, FPVec3::new(FP::ZERO, y, FP::ZERO), v3!(0.5, 0.5, 0.5)));
    }
    f.singleton_mut::<PhysicsState>().config = cfg;
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 900);
    let t = body(&f, es[(n - 1) as usize]);
    (t.pos.x.abs().max(t.pos.z.abs()), t.pos.y)
}

#[test]
fn sweep() {
    for (subs, iters) in [(8, 1), (6, 2), (12, 1), (16, 1), (6, 1), (8, 2)] {
        let mut line = String::new();
        for n in [15, 20, 25, 30, 40] {
            let mut c = PhysicsConfig::default();
            c.sleep_ticks = 0;
            c.substeps = subs;
            c.velocity_iterations = iters;
            let (l, _y) = stack(n, c);
            line.push_str(&format!("n{n}:{l} "));
        }
        println!("C subs {subs} it {iters}: {line}");
    }
}
