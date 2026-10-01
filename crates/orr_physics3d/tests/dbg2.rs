mod common;
use common::scenes::*;
use orr_physics3d::Scratch;

#[test]
fn cg() {
    let mut f = bench_pile(1000, false);
    let mut sc = Scratch::new();
    for _ in 0..140 {
        orr_physics3d::step(&mut f, &mut sc);
    }
}
