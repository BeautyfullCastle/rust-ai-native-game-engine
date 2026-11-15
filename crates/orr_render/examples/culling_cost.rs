//! CPU time of procedural culling: a grid of balls behind and beside a row of
//! walls, partitioned with frustum culling, then with occlusion as well.
//! Run with `cargo run --release -p orr_render --example culling_cost`.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]

use orr_render::visibility::{partition, Culling, View};
use orr_render::{Camera3D, Material, RenderList3D, IDENTITY_ROT};
use std::time::Instant;

fn scene(balls: usize) -> RenderList3D {
    let mut list = RenderList3D::new();
    let m = Material::new([0.5; 3]);
    list.plane([0.0; 3], 200.0, 200.0, &m);
    for i in 0..8 {
        list.cuboid([-28.0 + 8.0 * i as f32, 3.0, -10.0], IDENTITY_ROT, [3.5, 3.0, 0.3], &m);
    }
    let side = (balls as f32).sqrt() as usize;
    for i in 0..balls {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        list.sphere([-50.0 + 100.0 * x / side as f32, 1.0, -12.0 - 80.0 * z / side as f32], IDENTITY_ROT, 0.4, &m);
    }
    list
}

fn main() {
    let camera = Camera3D::perspective([0.0, 2.0, 10.0], [0.0, 1.5, 0.0], 60.0);
    let view = View { view_proj: camera.view_proj(16.0 / 9.0).0, eye: camera.eye, forward: camera.forward() };
    for balls in [1_000, 10_000, 50_000] {
        let list = scene(balls);
        let all = [&list.spheres[..], &list.boxes[..], &list.capsules[..], &list.planes[..]];
        for (name, culling) in [("frustum", Culling { frustum: true, occlusion: false }), ("frustum+occlusion", Culling::default())] {
            let runs = 20;
            let start = Instant::now();
            let mut visible = [0; 4];
            for _ in 0..runs {
                visible = partition(&view, culling, all).1;
            }
            let per = start.elapsed() / runs;
            println!("{balls:>6} balls  {name:<18} {per:>10.2?} per frame, {} of {} balls drawn", visible[0], balls);
        }
    }
}
