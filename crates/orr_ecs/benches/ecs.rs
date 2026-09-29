use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder, Frame};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Pos {
    x: i64,
    y: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vel {
    x: i64,
    y: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Extra {
    a: i64,
    b: i64,
}

fn registry() -> Arc<ComponentRegistry> {
    let mut b = ComponentRegistryBuilder::new();
    b.register_component::<Pos>("Pos");
    b.register_component::<Vel>("Vel");
    b.register_component::<Extra>("Extra");
    b.build()
}

fn bench_spawn(c: &mut Criterion) {
    let reg = registry();
    c.bench_function("spawn_100k_entities_2_components", |b| {
        b.iter_batched(
            || Frame::new(reg.clone()),
            |mut f| {
                for i in 0..100_000i64 {
                    let e = f.spawn();
                    f.add(e, Pos { x: i, y: i });
                    f.add(e, Vel { x: 1, y: 1 });
                }
                f
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_iterate_integrate(c: &mut Criterion) {
    let reg = registry();
    let mut f = Frame::new(reg);
    for i in 0..1_000_000i64 {
        let e = f.spawn();
        f.add(e, Pos { x: 0, y: 0 });
        f.add(e, Vel { x: i % 7, y: i % 5 });
    }
    c.bench_function("iterate_1m_integrate_pos_vel", |b| {
        b.iter(|| {
            f.query::<(&mut Pos, &Vel)>().for_each(|_, (p, v)| {
                p.x += v.x;
                p.y += v.y;
            });
        });
    });
}

fn bench_iterate_integrate_plain_vec(c: &mut Criterion) {
    let n = 1_000_000i64;
    let mut pos: Vec<Pos> = (0..n).map(|_| Pos { x: 0, y: 0 }).collect();
    let vel: Vec<Vel> = (0..n).map(|i| Vel { x: i % 7, y: i % 5 }).collect();
    c.bench_function("iterate_1m_integrate_pos_vel_plain_vec", |b| {
        b.iter(|| {
            for (p, v) in pos.iter_mut().zip(vel.iter()) {
                p.x += v.x;
                p.y += v.y;
            }
        });
    });
}

fn bench_add_remove(c: &mut Criterion) {
    let reg = registry();
    let mut f = Frame::new(reg);
    let entities: Vec<_> = (0..10_000).map(|_| f.spawn()).collect();
    c.bench_function("add_remove_component_10k", |b| {
        b.iter(|| {
            for &e in &entities {
                f.add(e, Extra { a: 1, b: 2 });
            }
            for &e in &entities {
                f.remove::<Extra>(e);
            }
        });
    });
}

fn bench_snapshot(c: &mut Criterion) {
    let reg = registry();
    let mut group = c.benchmark_group("snapshot_copy_from");
    for &n in &[10_000i64, 100_000i64] {
        let mut src = Frame::new(reg.clone());
        for i in 0..n {
            let e = src.spawn();
            src.add(e, Pos { x: i, y: i });
            src.add(e, Vel { x: 1, y: 1 });
            src.add(e, Extra { a: i, b: i });
        }
        let mut dst = Frame::new(reg.clone());
        group.bench_function(format!("{n}_entities_3_components"), |b| {
            b.iter(|| {
                dst.copy_from(&src);
            });
        });
    }
    group.finish();
}

fn bench_checksum(c: &mut Criterion) {
    let reg = registry();
    let mut f = Frame::new(reg);
    for i in 0..100_000i64 {
        let e = f.spawn();
        f.add(e, Pos { x: i, y: i });
        f.add(e, Vel { x: 1, y: 1 });
        f.add(e, Extra { a: i, b: i });
    }
    c.bench_function("checksum_100k", |b| {
        b.iter(|| f.checksum());
    });
}

fn bench_serialize(c: &mut Criterion) {
    let reg = registry();
    let mut group = c.benchmark_group("frame_bytes");
    for &n in &[10_000i64, 100_000i64] {
        let mut src = Frame::new(reg.clone());
        for i in 0..n {
            let e = src.spawn();
            src.add(e, Pos { x: i, y: i });
            src.add(e, Vel { x: 1, y: 1 });
            src.add(e, Extra { a: i, b: i });
        }
        let bytes = src.to_bytes();
        group.bench_function(format!("serialize_{n}_entities_3_components"), |b| {
            b.iter(|| src.to_bytes());
        });
        group.bench_function(format!("deserialize_{n}_entities_3_components"), |b| {
            b.iter(|| Frame::from_bytes(reg.clone(), &bytes).unwrap());
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_spawn,
    bench_iterate_integrate,
    bench_iterate_integrate_plain_vec,
    bench_add_remove,
    bench_snapshot,
    bench_checksum,
    bench_serialize
);
criterion_main!(benches);
