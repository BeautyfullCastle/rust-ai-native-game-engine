//! Bench: cost of one physics `step` with 1000 and 5000 dynamic bodies
//! (mixed circles, boxes and hexagons piling up in a walled arena), after
//! a warm-up that lets the pile form so contacts and warm starting are
//! active. Also the 8-tick resimulation a rollback would pay.
use criterion::{criterion_group, criterion_main, Criterion};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FPVec2, FrameRng, FP};
use orr_physics::{init, register, spawn_body, step, Body, Collider, PhysicsConfig, Scratch, Shape};

fn build(n: u32) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());

    // Arena width grows with sqrt(n) so pile height stays similar.
    let mut cols = 10i32;
    while (cols * cols) < n as i32 {
        cols += 1;
    }
    let half_w = FP::from_int(cols) * fp!(0.9);
    spawn_body(
        &mut f,
        Body::new_static(FPVec2::new(FP::ZERO, -FP::HALF), FP::ZERO),
        Collider::new(Shape::box_shape(half_w + fp!(2), FP::HALF)),
    );
    for sx in [-1, 1] {
        spawn_body(
            &mut f,
            Body::new_static(FPVec2::new(half_w * sx + FP::from_int(sx), fp!(100)), FP::ZERO),
            Collider::new(Shape::box_shape(FP::ONE, fp!(100))),
        );
    }
    let hex = {
        let pts: Vec<FPVec2> = (0..6)
            .map(|k| FPVec2::from_angle(FP::TWO_PI * FP::from_int(k) / FP::from_int(6)) * fp!(0.5))
            .collect();
        Shape::polygon(&pts).unwrap()
    };
    let mut rng = FrameRng::new(7);
    for i in 0..n as i32 {
        let col = i % cols;
        let row = i / cols;
        let x = FP::from_int(col * 2 - cols) * fp!(0.9) + rng.range_fp(-fp!(0.1), fp!(0.1));
        let y = fp!(1) + FP::from_int(row) * fp!(1.2);
        let pos = FPVec2::new(x, y);
        match i % 3 {
            0 => {
                let s = Shape::circle(fp!(0.4));
                spawn_body(&mut f, Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_restitution(fp!(0.2)));
            }
            1 => {
                let s = Shape::box_shape(fp!(0.4), fp!(0.4));
                spawn_body(&mut f, Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s));
            }
            _ => {
                spawn_body(&mut f, Body::new_dynamic(pos, &hex, FP::ONE), Collider::new(hex));
            }
        }
    }
    f
}

fn bench_step(c: &mut Criterion, n: u32) {
    let mut frame = build(n);
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    for _ in 0..300 {
        step(&mut frame, &mut sc, &mut events);
    }
    c.bench_function(&format!("physics_step_{n}_bodies"), |b| {
        b.iter(|| {
            events.clear();
            step(&mut frame, &mut sc, &mut events);
        })
    });
}

fn bench_resim(c: &mut Criterion) {
    let mut frame = build(1000);
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    for _ in 0..300 {
        step(&mut frame, &mut sc, &mut events);
    }
    let snapshot = frame.clone();
    c.bench_function("physics_rollback_8_ticks_1000_bodies", |b| {
        b.iter(|| {
            frame.copy_from(&snapshot);
            for _ in 0..8 {
                events.clear();
                step(&mut frame, &mut sc, &mut events);
            }
        })
    });
}

fn benches(c: &mut Criterion) {
    bench_step(c, 1000);
    bench_step(c, 5000);
    bench_resim(c);
}

criterion_group!(physics, benches);
criterion_main!(physics);
