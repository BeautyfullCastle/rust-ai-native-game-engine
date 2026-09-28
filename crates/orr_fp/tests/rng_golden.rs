//! Pins the exact PCG32 output sequence for a fixed seed, so a change to
//! `FrameRng`'s internals (accidental or not) is caught immediately.

use orr_fp::FrameRng;

const GOLDEN_SEQUENCE: [u32; 10] = [
    565663470, 3244226384, 2504567229, 903561869, 4026996297, 2722332799, 3032858066, 272411090,
    1181909318, 20290832,
];

#[test]
fn rng_golden_sequence() {
    let mut rng = FrameRng::new(42);
    let got: Vec<u32> = (0..10).map(|_| rng.next_u32()).collect();
    eprintln!("rng golden sequence = {got:?}");
    assert_eq!(got, GOLDEN_SEQUENCE);
}

#[test]
fn rng_same_seed_same_sequence() {
    let mut a = FrameRng::new(123);
    let mut b = FrameRng::new(123);
    for _ in 0..50 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn rng_different_seeds_diverge() {
    let mut a = FrameRng::new(1);
    let mut b = FrameRng::new(2);
    let seq_a: Vec<u32> = (0..20).map(|_| a.next_u32()).collect();
    let seq_b: Vec<u32> = (0..20).map(|_| b.next_u32()).collect();
    assert_ne!(seq_a, seq_b);
}

#[test]
fn rng_fork_diverges_from_parent() {
    let mut parent = FrameRng::new(999);
    let mut child = parent.fork(1);
    let parent_seq: Vec<u32> = (0..20).map(|_| parent.next_u32()).collect();
    let child_seq: Vec<u32> = (0..20).map(|_| child.next_u32()).collect();
    assert_ne!(parent_seq, child_seq);
}

#[test]
fn rng_range_i32_within_bounds() {
    let mut rng = FrameRng::new(7);
    for _ in 0..1000 {
        let v = rng.range_i32(-5, 5);
        assert!((-5..5).contains(&v), "v={v}");
    }
}

#[test]
fn rng_fp01_within_bounds() {
    let mut rng = FrameRng::new(7);
    for _ in 0..1000 {
        let v = rng.next_fp01();
        assert!(v >= orr_fp::FP::ZERO && v < orr_fp::FP::ONE, "v={v:?}");
    }
}
