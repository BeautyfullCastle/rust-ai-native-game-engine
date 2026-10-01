//! The wasm/narrow fast paths of `FP::mul`, `FP::div` and `FP::sqrt` against the plain
//! `i128`/`u128` formulas of the crate-level contract. The formulas below are the reference;
//! the operators must return the same bits for every input, on every target (this test runs
//! natively, on wasm32-wasip1, and on aarch64 under qemu in CI).

use orr_fp::{FrameRng, FP};

fn edge_values() -> Vec<i64> {
    let mut v = vec![0i64, 1, -1, 2, -2, 65_535, 65_536, -65_536, i64::MAX, i64::MIN, i64::MIN + 1, i64::MAX - 1];
    for k in 1..63 {
        for d in -2..=2i64 {
            v.push((1i64 << k) + d);
            v.push(-(1i64 << k) + d);
        }
    }
    v.extend([i32::MAX as i64, i32::MIN as i64, i32::MAX as i64 + 1, i32::MIN as i64 - 1, (1 << 47) - 1, 1 << 47, -(1 << 47), -(1 << 47) - 1]);
    v
}

fn random_values(seed: u64, n: usize) -> Vec<i64> {
    let mut rng = FrameRng::new(seed);
    (0..n)
        .map(|_| {
            let bits = rng.next_u32() % 64;
            let raw = (((rng.next_u32() as u64) << 32) | rng.next_u32() as u64) >> (63 - bits.min(63));
            if rng.next_u32() & 1 == 0 {
                raw as i64
            } else {
                (raw as i64).wrapping_neg()
            }
        })
        .collect()
}

fn mul_ref(a: i64, b: i64) -> i64 {
    (((a as i128) * (b as i128)) >> 16) as i64
}

fn div_ref(a: i64, b: i64) -> i64 {
    (((a as i128) << 16) / (b as i128)) as i64
}

fn sqrt_ref(raw: i64) -> i64 {
    // Bitwise floor square root of raw * 2^16, the contract's definition.
    let n = (raw as u128) << 16;
    let (mut res, mut bit, mut rem) = (0u128, 1u128 << 126, n);
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= res + bit {
            rem -= res + bit;
            res = (res >> 1) + bit;
        } else {
            res >>= 1;
        }
        bit >>= 2;
    }
    res as i64
}

#[test]
fn mul_matches_the_i128_formula() {
    let edges = edge_values();
    for &a in &edges {
        for &b in &edges {
            assert_eq!((FP(a) * FP(b)).0, mul_ref(a, b), "{a} * {b}");
            assert_eq!(FP(a).mul(FP(b)).0, mul_ref(a, b), "{a}.mul({b})");
        }
    }
    for w in random_values(5, 600_000).chunks(2) {
        assert_eq!((FP(w[0]) * FP(w[1])).0, mul_ref(w[0], w[1]), "{} * {}", w[0], w[1]);
    }
    // Operands of game magnitude (the narrow path) and the boundary of it.
    for w in random_values(8, 200_000).chunks(2) {
        let (a, b) = (w[0] >> 32, w[1] >> 31);
        assert_eq!((FP(a) * FP(b)).0, mul_ref(a, b), "{a} * {b}");
    }
}

#[test]
fn div_matches_the_i128_formula() {
    let edges = edge_values();
    for &a in &edges {
        for &b in edges.iter().filter(|&&b| b != 0) {
            // The i128 numerator never overflows, but the quotient may exceed i64 (it then wraps
            // the same way in both).
            assert_eq!((FP(a) / FP(b)).0, div_ref(a, b), "{a} / {b}");
        }
    }
    for w in random_values(6, 600_000).chunks(2) {
        if w[1] != 0 {
            assert_eq!((FP(w[0]) / FP(w[1])).0, div_ref(w[0], w[1]), "{} / {}", w[0], w[1]);
        }
    }
}

#[test]
fn sqrt_matches_the_bitwise_formula() {
    let check = |raw: i64| assert_eq!(FP(raw).sqrt().0, sqrt_ref(raw), "sqrt of raw {raw}");
    for raw in 0..300_000i64 {
        check(raw);
    }
    for k in 0..63 {
        for d in -4..=4i64 {
            check(((1i64 << k) + d).max(0));
        }
    }
    check(i64::MAX);
    check((1 << 48) - 1);
    check(1 << 48);
    for raw in random_values(99, 600_000) {
        check(raw.wrapping_abs().max(0));
    }
    // Perfect squares and their neighbours (the correction loops).
    for s in (1u64..(1 << 24)).step_by(61) {
        let sq = ((s * s) >> 16) as i64;
        for d in -2..=2 {
            check((sq + d).max(0));
        }
    }
}
