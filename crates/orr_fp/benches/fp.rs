#![allow(clippy::float_arithmetic)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use orr_fp::{fp, FPVec3, FP};

fn bench_fp(c: &mut Criterion) {
    let a = fp!(1.2345);
    let b = fp!(0.6789);
    let af = 1.2345_f32;
    let bf = 0.6789_f32;

    c.bench_function("fp_mul", |bch| bch.iter(|| black_box(a) * black_box(b)));
    c.bench_function("fp_mul_fast", |bch| bch.iter(|| black_box(a).mul_fast(black_box(b))));
    c.bench_function("f32_mul", |bch| bch.iter(|| black_box(af) * black_box(bf)));

    c.bench_function("fp_div", |bch| bch.iter(|| black_box(a) / black_box(b)));
    c.bench_function("f32_div", |bch| bch.iter(|| black_box(af) / black_box(bf)));

    let big = fp!(1234.5678);
    let bigf = 1234.5678_f32;
    c.bench_function("fp_sqrt", |bch| bch.iter(|| black_box(big).sqrt()));
    c.bench_function("f32_sqrt", |bch| bch.iter(|| black_box(bigf).sqrt()));

    let angle = fp!(0.78539816339745);
    let anglef = std::f32::consts::FRAC_PI_4;
    c.bench_function("fp_sin", |bch| bch.iter(|| black_box(angle).sin()));
    c.bench_function("f32_sin", |bch| bch.iter(|| black_box(anglef).sin()));
    c.bench_function("fp_cos", |bch| bch.iter(|| black_box(angle).cos()));
    c.bench_function("fp_sin_cos", |bch| bch.iter(|| black_box(angle).sin_cos()));
    c.bench_function("fp_tan", |bch| bch.iter(|| black_box(angle).tan()));

    // Varying angles across the full turn, so a table lookup cannot hide
    // cache misses behind a single hot entry.
    let mut seed = 0x9e3779b97f4a7c15_u64;
    let sweep: Vec<FP> = (0..4096)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            FP::from_raw(((seed >> 33) as i64 % 411_775) - 205_887)
        })
        .collect();
    c.bench_function("fp_sin_sweep", |bch| {
        bch.iter(|| {
            let mut acc = FP::ZERO;
            for &a in &sweep {
                acc += black_box(a).sin();
            }
            acc
        })
    });

    c.bench_function("fp_atan", |bch| bch.iter(|| black_box(a).atan()));
    c.bench_function("fp_atan_cordic", |bch| bch.iter(|| black_box(a).atan_cordic()));
    c.bench_function("fp_atan2", |bch| bch.iter(|| black_box(a).atan2(black_box(b))));
    c.bench_function("fp_atan2_cordic", |bch| {
        bch.iter(|| black_box(a).atan2_cordic(black_box(b)))
    });
    // Varying (y, x) pairs of mixed magnitude and sign (game-like: up to
    // a few hundred units), so both the `|y/x| <= 1` and `> 1` branches run.
    let pairs: Vec<(FP, FP)> = (0..4096)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let y = ((seed >> 33) as i64 % 40_000_000) - 20_000_000;
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let x = ((seed >> 33) as i64 % 40_000_000) - 20_000_000;
            (FP::from_raw(y), FP::from_raw(x | 1))
        })
        .collect();
    let ratios: Vec<FP> = pairs.iter().map(|&(y, x)| y / x).collect();
    c.bench_function("fp_atan_sweep", |bch| {
        bch.iter(|| {
            let mut acc = FP::ZERO;
            for &t in &ratios {
                acc += black_box(t).atan();
            }
            acc
        })
    });
    c.bench_function("fp_atan2_sweep", |bch| {
        bch.iter(|| {
            let mut acc = FP::ZERO;
            for &(y, x) in &pairs {
                acc += black_box(y).atan2(black_box(x));
            }
            acc
        })
    });
    c.bench_function("f32_atan2", |bch| bch.iter(|| black_box(af).atan2(black_box(bf))));

    let v = FPVec3::new(fp!(1.0), fp!(2.0), fp!(3.0));
    c.bench_function("fp_vec3_normalize", |bch| bch.iter(|| black_box(v).normalize()));

    let vf = (1.0_f32, 2.0_f32, 3.0_f32);
    c.bench_function("f32_vec3_normalize", |bch| {
        bch.iter(|| {
            let (x, y, z) = black_box(vf);
            let len = (x * x + y * y + z * z).sqrt();
            (x / len, y / len, z / len)
        })
    });

    let _: FP = a; // silence unused import edge cases when features differ
}

criterion_group!(benches, bench_fp);
criterion_main!(benches);
