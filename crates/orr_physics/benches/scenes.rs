//! Scene builders shared by the criterion bench and the profile example.
#![allow(dead_code)]
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FPVec2, FrameRng, FP};
use orr_physics::{init, register, spawn_body, Body, Collider, PhysicsConfig, Shape, BODY_KINEMATIC};

/// Walled arena with mixed bodies dropped in a grid (piles up, then rests).
pub fn build(n: u32) -> Frame {
    build_scene(n, false)
}

/// Same arena, but the floor is a kinematic plate that shakes up and down
/// (drive it with [`shake`] before every step). The pile keeps bouncing, so
/// nothing ever falls asleep: the all-awake worst case.
pub fn build_shaker(n: u32) -> Frame {
    build_scene(n, true)
}

/// Sets the plate velocity so its position follows `A * sin(2 pi f t)`
/// (3 Hz, about 5 g peak) after tick `tick`. Call before `step`.
pub fn shake(frame: &mut Frame, tick: u64) {
    let dt = FP::from_ratio(1, 60);
    let phase = FP::TWO_PI * FP::from_int((tick % 20) as i32) / FP::from_int(20);
    let target_y = fp!(0.15) * phase.sin_cos().0 - FP::HALF;
    for (_, (b,)) in frame.query::<(&mut Body,)>() {
        if b.kind == BODY_KINEMATIC {
            b.vel = FPVec2::new(FP::ZERO, (target_y - b.pos.y) / dt);
        }
    }
}

fn build_scene(n: u32, shaker: bool) -> Frame {
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
    let floor = if shaker {
        Body::new_kinematic(FPVec2::new(FP::ZERO, -FP::HALF))
    } else {
        Body::new_static(FPVec2::new(FP::ZERO, -FP::HALF), FP::ZERO)
    };
    spawn_body(&mut f, floor, Collider::new(Shape::box_shape(half_w + fp!(2), FP::HALF)));
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



/// A wide floor with `n` mixed bodies dropped in stacks of 4 spread 3.5 apart:
/// the "typical game" load. Everything comes to rest and can sleep.
pub fn build_field(n: u32) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());
    let cols = (n as i32 + 3) / 4;
    let spacing = fp!(3.5);
    let half_w = FP::from_int(cols) * spacing / 2 + fp!(2);
    spawn_body(
        &mut f,
        Body::new_static(FPVec2::new(FP::ZERO, -FP::HALF), FP::ZERO),
        Collider::new(Shape::box_shape(half_w + fp!(2), FP::HALF)),
    );
    for sx in [-1, 1] {
        spawn_body(
            &mut f,
            Body::new_static(FPVec2::new((half_w + fp!(1.5)) * sx, fp!(20)), FP::ZERO),
            Collider::new(Shape::box_shape(FP::ONE, fp!(20))),
        );
    }
    let hex = {
        let pts: Vec<FPVec2> = (0..6)
            .map(|k| FPVec2::from_angle(FP::TWO_PI * FP::from_int(k) / FP::from_int(6)) * fp!(0.5))
            .collect();
        Shape::polygon(&pts).unwrap()
    };
    let mut rng = FrameRng::new(21);
    for i in 0..n as i32 {
        let (col, row) = (i % cols, i / cols);
        let x = FP::from_int(col - cols / 2) * spacing + rng.range_fp(-fp!(0.15), fp!(0.15));
        let y = fp!(1) + FP::from_int(row) * fp!(1.3);
        let pos = FPVec2::new(x, y);
        let s = match i % 3 {
            0 => Shape::circle(fp!(0.4)),
            1 => Shape::box_shape(fp!(0.4), fp!(0.4)),
            _ => hex,
        };
        let mut body = Body::new_dynamic(pos, &s, FP::ONE);
        body.linear_damping = fp!(0.05);
        body.angular_damping = fp!(0.3);
        let mut col = Collider::new(s);
        if i % 3 == 0 {
            col = col.with_restitution(fp!(0.2));
        }
        spawn_body(&mut f, body, col);
    }
    f
}
