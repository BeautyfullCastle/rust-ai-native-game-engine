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

    c.bench_function("fp_atan2", |bch| bch.iter(|| black_box(a).atan2(black_box(b))));
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
