mod common;
use common::*;
use orr_fp::{fp, FP};
use orr_physics3d::{spawn_body, Body, Collider, Scratch, Shape};

#[test]
fn sphere_edge() {
    let mut f = new_frame();
    ground(&mut f);
    spawn_body(&mut f, Body::new_static(v3!(0, 0.5, 0)), Collider::new(Shape::cuboid(fp!(1), fp!(0.5), fp!(1))));
    let s = spawn_sphere(&mut f, v3!(1.02, 1.6, 0), fp!(0.3));
    let mut sc = Scratch::new();
    for t in 0..12 {
        run(&mut f, &mut sc, 10);
        let b = body(&f, s);
        println!("t{} pos {:?} v {:?} w {:?} sleep {:#x} {:?}", t, b.pos, b.vel, b.omega, b.sleep, sc.stats());
    }
    let _ = FP::ONE;
}
